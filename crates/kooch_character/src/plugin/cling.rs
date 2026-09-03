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
use kooch_gravity::gravity_up;
use kooch_physics::plugin::{PhysicsWorld, SolverBody};

use crate::grounded::Grounded;
use crate::jump::{Jump, WallJump};
use crate::plugin::leap::{self, Leap, Tallies};
use crate::touching::Touching;
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
    steering: Vec3,
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
    let mut spent = Vec::new();
    for one in &asked {
        if let Some(slide) = one.slide {
            cling(&mut world, one, &slide);
        }
        if let Some(jump) = one.jump {
            let mut tally = tallies.of(one.entity);
            let wall = one.off.as_ref().zip(one.wall);
            let leap = leap::spend(&mut tally, &jump, wall, one.standing, one.up, dt);
            tallies.set(one.entity, tally);
            if let Some(leap) = leap {
                launch(&mut world, one, leap);
            }
            spent.push(one.entity);
        }
    }
    resources.insert(world);
    resources.insert(tallies);

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

/// Holds the fall to the slide speed, while the wall is being held on to.
fn cling(world: &mut PhysicsWorld, one: &Asked, slide: &WallSlide) {
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
    let falling = velocity.dot(one.up);
    if falling >= -slide.max_fall.max(0.0) {
        return;
    }
    // Only the part along up is replaced. Clamping the whole vector
    // would take the character's run at the wall away with it.
    let held = velocity - one.up * falling - one.up * slide.max_fall.max(0.0);
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
        // A wall jump replaces the horizontal too: it is a push away
        // from something, and keeping the run *into* the wall would
        // cancel most of it.
        Leap::Wall(away) => away,
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
            (jump.is_some() || slide.is_some()).then_some(())?;
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
            |(entity, body, position, standing, wall, jump, off, slide, steering)| Asked {
                entity,
                body,
                up: gravity_up(resources, position),
                standing,
                wall,
                jump,
                off,
                slide,
                steering,
            },
        )
        .collect()
}
