//! Slowing a fall down a wall, and spending wall and air jumps.
//!
//! Both read what the sense pass already found. Neither casts: a
//! mechanic that probes for itself is a mechanic that can disagree with
//! `Grounded` about whether the character is in the air.

use glam::Vec3;

use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::transform::Transform;
use kooch_gravity::{gravity_at, gravity_up};
use kooch_physics::plugin::{PhysicsWorld, SolverBody};

use crate::grounded::Grounded;
use crate::jump::{Jump, WallJump};
use crate::plugin::leap::{self, Leap, Tallies};
use crate::plugin::run::{self, Run, Runs};
use crate::touching::Touching;
use crate::wall_run::WallRun;
use crate::wall_slide::WallSlide;

/// One character's worth, read before the world is borrowed.
struct Asked {
    entity: Entity,
    body: SolverBody,
    up: Vec3,
    standing: bool,
    wall: Option<Vec3>,
    jump: Option<Jump>,
    off: Option<WallJump>,
    slide: Option<WallSlide>,
    run: Option<WallRun>,
    steering: Vec3,
    /// How hard the field pulls here, which a run holds off a fraction
    /// of.
    weight: f32,
}

impl Asked {
    /// The wall's normal, or `fallback` where there is no wall.
    fn normal_or(&self, fallback: Vec3) -> Vec3 {
        self.wall.unwrap_or(fallback)
    }
}

/// Caps the fall on a wall, then spends whatever jump was asked for.
pub fn cling_and_leap(resources: &mut Resources) {
    let asked = plan(resources);
    if asked.is_empty() {
        return;
    }
    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0);

    let Some(mut world) = resources.remove::<PhysicsWorld>() else {
        return;
    };
    let mut tallies = resources.remove::<Tallies>().unwrap_or_default();
    let mut runs = resources.remove::<Runs>().unwrap_or_default();
    let mut spent = Vec::new();
    for one in &asked {
        let mut tally = tallies.of(one.entity);
        // Not while it is getting clear of a wall it just pushed off:
        // the wall is still right there, and holding on would cancel
        // the jump that was just made.
        let clearing = tally.since_wall.is_some_and(|since| since < leap::CLEARING);
        // Off the wall, or back on the ground: the next wall is a fresh
        // question. Without this a refusal follows the character to
        // every wall it ever touches again.
        if one.standing || one.wall.is_none() {
            runs.landed(one.entity);
        }
        // A run first: a character that is running along a wall is not
        // sliding down it, and applying both is one asking to fall and
        // one asking not to.
        let running = match (one.run, one.wall, clearing) {
            (Some(spec), Some(normal), false) if !one.standing => {
                sprint_along(&mut world, &mut runs, one, &spec, normal, dt)
            }
            _ => false,
        };
        if let Some(slide) = one.slide
            && !clearing
            && !running
        {
            cling(&mut world, one, &slide, dt);
        }
        if let Some(jump) = one.jump {
            let wall = one.off.as_ref().zip(one.wall);
            let leap = leap::spend(&mut tally, &jump, wall, one.standing, one.up, dt);
            if let Some(leap) = leap {
                launch(&mut world, one, leap);
            }
            spent.push(one.entity);
        } else if let Some(since) = tally.since_wall.as_mut() {
            // Advanced even without a jump, or a character with a slide
            // and no wall jump would carry a timer that never moves.
            *since += dt;
        }
        tallies.set(one.entity, tally);
    }
    resources.insert(world);
    resources.insert(tallies);
    resources.insert(runs);

    // Cleared here rather than by gameplay: whoever set it cannot know
    // whether it was honoured, buffered or refused, and a flag left set
    // fires again on the next frame that allows it.
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<Jump>() else {
        return;
    };
    for entity in spent {
        if let Some(jump) = storage.get_mut(entity) {
            jump.wanted = false;
        }
    }
}

/// Carries the character along the wall while the clock allows, and
/// reports whether it did.
fn sprint_along(
    world: &mut PhysicsWorld,
    runs: &mut Runs,
    one: &Asked,
    spec: &WallRun,
    normal: Vec3,
    dt: f32,
) -> bool {
    let Some(velocity) = world.linear_velocity(one.body) else {
        return false;
    };
    let speed = run::along(velocity, normal, one.up).length();
    let carried = run::carry(runs.state(one.entity), speed, spec, dt);
    runs.set(one.entity, carried);
    let Run::Going(_) = carried else {
        return false;
    };

    let mut held = velocity;
    // The bounce off the wall, dropped, and held on in its place — the
    // same argument as the slide's `stick`.
    let out = held.dot(normal);
    if out > 0.0 {
        held -= normal * out;
    }
    held -= normal * spec.stick.max(0.0) * dt;
    // Gravity held off, not cancelled: the sag is what tells a player
    // the run is ending before it ends.
    held += one.up * one.weight * spec.hold.clamp(0.0, 1.0) * dt;
    world.set_linear_velocity(one.body, held);
    true
}

/// Holds the character on the wall, and its fall to the slide speed.
fn cling(world: &mut PhysicsWorld, one: &Asked, slide: &WallSlide, dt: f32) {
    let (false, Some(normal)) = (one.standing, one.wall) else {
        return;
    };
    // Steered into it, not merely beside it. Without this a character
    // running past a wall is slowed by it.
    let into = -one.steering.normalize_or_zero().dot(normal);
    if into < slide.grip.clamp(0.0, 1.0) {
        return;
    }
    let Some(velocity) = world.linear_velocity(one.body) else {
        return;
    };
    let mut held = velocity;

    // The bounce, dropped. Arriving at speed the solver pushes the
    // capsule back out, and with the air push deliberately not aimed
    // into the wall there is nothing to bring it back: the character
    // rebounds and drifts off mid-slide.
    let out = held.dot(one.normal_or(Vec3::ZERO));
    if out > 0.0 {
        held -= one.normal_or(Vec3::ZERO) * out;
    }
    // And held on, which the contact friction used to do by accident.
    held -= one.normal_or(Vec3::ZERO) * slide.stick.max(0.0) * dt;

    // Only the part along up is capped. Clamping the whole vector would
    // take the character's run along the wall away with it.
    let falling = held.dot(one.up);
    if falling < -slide.max_fall.max(0.0) {
        held = held - one.up * falling - one.up * slide.max_fall.max(0.0);
    }
    world.set_linear_velocity(one.body, held);
}

/// Sets the velocity a jump asks for, keeping what is across it.
///
/// Set rather than added: a jump taken while already falling would
/// otherwise be worth less than one taken at rest, and a player cannot
/// see their own vertical speed to allow for it.
fn launch(world: &mut PhysicsWorld, one: &Asked, leap: Leap) {
    let Some(velocity) = world.linear_velocity(one.body) else {
        return;
    };
    let launched = match leap {
        Leap::Ground(up) => velocity - one.up * velocity.dot(one.up) + up,
        // The speed *along* the wall is kept and the speed *into* it is
        // not. Keeping everything would spend most of the push undoing
        // the run at the wall; keeping nothing throws away the run
        // along it, which on a wall run is the whole point.
        Leap::Wall(away) => match one.wall {
            Some(normal) => run::along(velocity, normal, one.up) + away,
            None => away,
        },
    };
    world.set_linear_velocity(one.body, launched);
}

/// Everything asking, with what the world will need.
fn plan(resources: &Resources) -> Vec<Asked> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let (Some(bodies), Some(transforms), Some(grounded)) = (
        registry.get_cpu::<SolverBody>(),
        registry.get_cpu::<Transform>(),
        registry.get_cpu::<Grounded>(),
    ) else {
        return Vec::new();
    };
    let touching = registry.get_cpu::<Touching>();
    let jumps = registry.get_cpu::<Jump>();
    let offs = registry.get_cpu::<WallJump>();
    let slides = registry.get_cpu::<WallSlide>();
    let running = registry.get_cpu::<WallRun>();
    let facings = registry.get_cpu::<crate::facing::Facing>();

    type Read = (
        Entity,
        SolverBody,
        Vec3,
        bool,
        Option<Vec3>,
        Option<Jump>,
        Option<WallJump>,
        Option<WallSlide>,
        Option<WallRun>,
        Vec3,
    );
    let asked: Vec<Read> = grounded
        .iter()
        .filter_map(|(&entity, found)| {
            let jump = jumps.and_then(|jumps| jumps.get(entity)).copied();
            let slide = slides.and_then(|slides| slides.get(entity)).copied();
            // Nothing to do for a character with neither. Either alone
            // is enough: a wall to slide down is a mechanic without a
            // jump, and a jump is one without a wall.
            let run = running.and_then(|running| running.get(entity)).copied();
            (jump.is_some() || slide.is_some() || run.is_some()).then_some(())?;
            Some((
                entity,
                *bodies.get(entity)?,
                transforms.get(entity)?.position,
                found.standing,
                touching
                    .and_then(|touching| touching.get(entity))
                    .filter(|touching| touching.wall)
                    .map(|touching| touching.normal),
                jump,
                offs.and_then(|offs| offs.get(entity)).copied(),
                slide,
                run,
                facings
                    .and_then(|facings| facings.get(entity))
                    .map(|facing| facing.direction)
                    .unwrap_or(Vec3::ZERO),
            ))
        })
        .collect();

    asked
        .into_iter()
        .map(
            |(entity, body, position, standing, wall, jump, off, slide, run, steering)| Asked {
                entity,
                body,
                up: gravity_up(resources, position),
                standing,
                wall,
                jump,
                off,
                slide,
                run,
                steering,
                weight: gravity_at(resources, position).length(),
            },
        )
        .collect()
}
