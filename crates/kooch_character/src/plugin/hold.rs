//! The sweep, the walk, the turn and the spring — one step of a
//! character, in the order they depend on each other.

use glam::{Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::transform::Transform;
use kooch_gravity::gravity_at;
use kooch_physics::plugin::{PhysicsWorld, SolverBody};

use crate::controller::CharacterController;
use crate::facing::Facing;
use crate::grounded::Grounded;
use crate::plugin::run::{self, Runs};
use crate::plugin::sense::{self};
use crate::plugin::{turn, walk};
use crate::sprint::Sprint;
use crate::touching::Touching;
use crate::walk::Walk;

/// Below this throttle a character is standing still, which is what ends a toggled run.
const STILL: f32 = 0.05;

/// Advances every sprint one step against whether its character is steered anywhere.
fn step_sprints(resources: &mut Resources) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let moving: Vec<(Entity, bool)> = registry
        .get_cpu::<Facing>()
        .map(|facings| {
            facings
                .iter()
                .map(|(&entity, facing)| (entity, facing.direction.length() > STILL))
                .collect()
        })
        .unwrap_or_default();
    let Some(sprints) = registry.get_cpu_mut::<Sprint>() else {
        return;
    };
    for (&entity, sprint) in sprints.iter_mut() {
        let moving = moving.iter().any(|&(e, m)| e == entity && m);
        sprint.step(moving);
    }
}

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
    /// Whether it is running, and by how much.
    sprint: Sprint,
    /// Whether this character is interested in walls at all.
    walls: bool,
    /// How far to bank while running a wall, and which way, or `None` — from the previous step's
    /// run, a frame of lag nobody sees.
    bank: Option<(Vec3, f32)>,
}

/// Rising faster than this, in m/s, the spring lets go — otherwise its damping fights a jump (at
/// 18, a 5 m/s launch meets 90 m/s² back).
const RISING: f32 = 0.5;

/// Sweeps for ground, holds the body at its ride height, keeps it
/// upright, and writes [`Grounded`].
pub fn hold_characters(resources: &mut Resources) {
    step_sprints(resources);
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
    let found: Vec<(Entity, Grounded, Touching)> = planned
        .iter()
        .map(|plan| {
            let (grounded, touching) = hold_one(&mut world, &mut goals, plan, dt);
            (plan.entity, grounded, touching)
        })
        .collect();
    resources.insert(world);
    resources.insert(goals);

    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    if let Some(storage) = registry.get_cpu_mut::<Grounded>() {
        for (entity, grounded, _) in &found {
            storage.insert(*entity, *grounded);
        }
    }
    // Only where one is authored: a character that has no use for walls
    // should not grow the component by being simulated.
    if let Some(storage) = registry.get_cpu_mut::<Touching>() {
        for (entity, _, touching) in &found {
            if storage.get(*entity).is_some() {
                storage.insert(*entity, *touching);
            }
        }
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
    let sprints = registry.get_cpu::<Sprint>();
    let touched = registry.get_cpu::<Touching>();
    let runs = resources.get::<Runs>();
    let running = registry.get_cpu::<crate::wall_run::WallRun>();

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
        Sprint,
        bool,
        Option<(Vec3, f32)>,
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
                sprints
                    .and_then(|sprints| sprints.get(entity))
                    .copied()
                    .unwrap_or(Sprint {
                        wanted: false,
                        ..Default::default()
                    }),
                touched.is_some_and(|touched| touched.get(entity).is_some()),
                runs.and_then(|runs| runs.of(entity))
                    .and(running.and_then(|running| running.get(entity)))
                    .zip(touched.and_then(|touched| touched.get(entity)))
                    .filter(|(_, touched)| touched.wall)
                    .map(|(spec, touched)| (touched.normal, spec.bank)),
            ))
        })
        .collect();

    standing
        .into_iter()
        .map(
            |(entity, body, controller, position, rotation, facing, walk, sprint, walls, bank)| {
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
                    sprint,
                    walls,
                    bank,
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
) -> (Grounded, Touching) {
    let controller = &plan.controller;
    let filter = world.without(plan.body);
    let under = sense::under(world, controller, plan.position, plan.up, filter);
    let standing = under
        .as_ref()
        .map(|under| under.footing.stands())
        .unwrap_or(false);

    // Where it is steering, or going when nothing is asked: a body pressed to a wall barely moves
    // into it, exactly when a slide needs to see it.
    let velocity = world.linear_velocity(plan.body).unwrap_or(Vec3::ZERO);
    let along = match plan.facing.length_squared() > 1e-6 {
        true => plan.facing,
        false => velocity,
    };
    // Ahead and to both sides, only for a character that has a `Touching` — looking ahead never
    // finds the wall being run along.
    let found = plan
        .walls
        .then(|| sense::beside(world, controller, plan.position, along, plan.up, filter))
        .flatten();
    let touching = match found {
        Some((normal, distance)) => Touching {
            wall: true,
            normal,
            distance,
        },
        None => Touching::default(),
    };

    let across = velocity - plan.up * velocity.dot(plan.up);
    // Measured before the walk pushes, so it is the speed the last step
    // actually produced rather than the force this one is about to ask
    // for. A body shoving a wall is given everything and goes nowhere.
    let gained = goals.gained(plan.entity, across, dt);
    walk_one(
        world,
        goals,
        plan,
        standing,
        touching.wall.then_some(touching.normal),
        dt,
    );
    turn_one(world, plan, gained, dt);

    let Some(under) = under else {
        return (Grounded::default(), touching);
    };
    let gap = (plan.position - under.point).dot(plan.up);

    // Too steep with nothing to arrive at: reported for animation and left to gravity, or the
    // spring would carry the character up a cliff.
    if !under.footing.holds() {
        return (
            Grounded {
                standing: false,
                normal: under.normal,
                distance: gap,
            },
            touching,
        );
    }

    let velocity = world.linear_velocity(plan.body).unwrap_or(Vec3::ZERO);
    // Measured along the surface, not the field: walking up a ramp read as leaving the ground, so a
    // character on a slope could not jump.
    let leaving = velocity.dot(under.normal.normalize_or(plan.up));
    // Leaving the ground under its own power. The spring would spend the
    // next frames pulling it straight back down, which is a jump that
    // never happens — see `RISING`.
    if leaving > RISING {
        return (
            Grounded {
                standing: false,
                normal: under.normal,
                distance: gap,
            },
            touching,
        );
    }
    let speed = velocity.dot(plan.up);

    // The spring pulls both ways, so the character follows the ground down a bump. Measured to the
    // contact point: on a slope the sweep's `t` understates the gap.
    let error = controller.ride_height - gap;
    // Gravity is cancelled first, or the spring leans to hold the body and the rest height drifts
    // with every planet.
    let acceleration = plan.weight + error * controller.stiffness - speed * controller.damping;
    if let Some(mass) = world.mass(plan.body) {
        world.apply_impulse(plan.body, plan.up * acceleration * mass * dt);
    }

    (
        Grounded {
            standing,
            normal: under.normal,
            distance: gap,
        },
        touching,
    )
}

/// Chases the goal velocity.
fn walk_one(
    world: &mut PhysicsWorld,
    goals: &mut walk::WalkGoals,
    plan: &Planned,
    standing: bool,
    wall: Option<Vec3>,
    dt: f32,
) {
    let Some(mut steps) = plan.walk else {
        return;
    };
    // A sprint is walking with two numbers scaled, so it is applied here
    // rather than by a system of its own — see `Sprint`.
    let (speed, eagerness) = plan.sprint.scale();
    steps.max_speed *= speed;
    steps.acceleration *= eagerness;
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
            let push = walk::drift(plan.facing, across, plan.up, &steps, dt);
            // Never into a wall: the contact friction alone holds a character at 0.8 m/s² of fall —
            // sliding is `WallSlide`'s call.
            walk::alongside(push, wall)
        }
    };

    if let Some(mass) = world.mass(plan.body) {
        world.apply_impulse(plan.body, pushed * mass * dt);
    }
}

/// Stands the body on the local up, facing its steering — set with angular velocity zeroed, or the
/// solver spins it back out of the pose.
fn turn_one(world: &mut PhysicsWorld, plan: &Planned, gained: Vec3, dt: f32) {
    let lean = plan.walk.map(|steps| steps.lean).unwrap_or(0.0);
    // Upright against the field, not the ground, so crossing a ramp does not tip the body sideways.
    let up = turn::leaned(plan.up, gained, plan.weight, lean);
    // Banked towards the wall while running one. Upright, a character
    // running along a wall reads as one hovering beside it.
    let up = match plan.bank {
        Some((normal, bank)) => run::banked(up, normal, bank),
        None => up,
    };
    let wanted = turn::wanted(up, plan.facing, plan.rotation);
    let turned = turn::towards(plan.rotation, wanted, plan.controller.turn_speed, dt);
    if turned.abs_diff_eq(plan.rotation, 1e-6) {
        return;
    }
    world.set_rotation(plan.body, turned);
    world.set_angular_velocity(plan.body, Vec3::ZERO);
}
