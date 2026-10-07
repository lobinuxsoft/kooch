//! A landing fires an impulse (#1419).
//!
//! 🔴 **A landing is not a contact.** The capsule floats — `Grounded::distance` says so itself,
//! *"not zero while standing, since the spring holds it open"* — so the solver never reports a
//! touch with the floor and `ContactForce` never fires. The edge of `Grounded.standing` going true
//! is what a landing actually is.
//!
//! It publishes [`Impulse`] rather than reaching into a camera. A landing is a fact about the
//! world: a sound, a dust puff and a controller rumble will want it too, and none of them should
//! have to learn what a camera is.

use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::impulse::{Impulse, ImpulseSource};
use kooch_ecs::reflect::FieldRange;
use kooch_physics::PhysicsWorld;
use kooch_physics::SolverBody;

use crate::grounded::Grounded;

/// Fires this character's [`ImpulseSource`] when it lands, scaled by how fast it was falling.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Impulse")]
pub struct LandingImpulse {
    /// Below this fall speed, nothing. Keeps walking down a ramp from shaking the camera.
    #[reflect(range = SPEED_RANGE)]
    pub min_speed: f32,
    /// The fall speed at which the source's amplitude is felt in full. Faster is clamped here.
    #[reflect(range = SPEED_RANGE)]
    pub full_speed: f32,
    /// The fastest this character has been falling since it left the ground. Written every step;
    /// authoring it does nothing.
    ///
    /// 🔴 The PEAK rather than the speed on the frame of contact. The spring has already begun
    /// pushing back by the time `standing` turns true, so reading the velocity at that instant
    /// measures the spring and not the fall.
    #[reflect(skip)]
    pub fall_speed: f32,
    /// Last step's `standing`, so a landing is an edge rather than a state.
    #[reflect(skip)]
    pub was_standing: bool,
}

const SPEED_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

impl Default for LandingImpulse {
    fn default() -> Self {
        Self {
            // Stepping off a kerb is about 2 m/s; a jump lands around 7.
            min_speed: 3.0,
            full_speed: 15.0,
            fall_speed: 0.0,
            was_standing: true,
        }
    }
}

impl Component for LandingImpulse {}

/// What one character's landing is worth, planned before anything is written.
struct Landed {
    entity: Entity,
    strength: f32,
}

/// Watches every character for the moment it arrives, and fires its impulse.
pub fn impulses_from_landings(resources: &mut Resources) {
    let (landed, falling) = planned(resources);
    remember(resources, &falling);
    if landed.is_empty() {
        return;
    }
    let fired = sourced(resources, &landed);
    if let Some(events) = resources.get_mut::<kooch_core::event::Events<Impulse>>() {
        for impulse in fired {
            events.send(impulse);
        }
    }
}

/// Who landed this step, and what every character's fall speed now is.
fn planned(resources: &Resources) -> (Vec<Landed>, Vec<(Entity, f32, bool)>) {
    let (Some(registry), Some(world)) = (
        resources.get::<ComponentRegistry>(),
        resources.get::<PhysicsWorld>(),
    ) else {
        return (Vec::new(), Vec::new());
    };
    let (Some(landings), Some(grounded), Some(bodies)) = (
        registry.get_cpu::<LandingImpulse>(),
        registry.get_cpu::<Grounded>(),
        registry.get_cpu::<SolverBody>(),
    ) else {
        return (Vec::new(), Vec::new());
    };

    let mut landed = Vec::new();
    let mut falling = Vec::new();
    for (&entity, landing) in landings.iter() {
        let Some(ground) = grounded.get(entity) else {
            continue;
        };
        let down = bodies
            .get(entity)
            .and_then(|body| world.handle(body.slot()))
            .and_then(|handle| world.backend().linear_velocity(handle))
            // 🔴 Against the surface's own normal, not world down: landing on the inside of a
            // sphere is still landing, and on a planet "down" is wherever you are standing.
            .map(|velocity| -velocity.dot(ground.normal.normalize_or(glam::Vec3::Y)))
            .unwrap_or(0.0);

        let peak = match ground.standing {
            // On the ground the record resets, so the next fall starts from nothing.
            true => 0.0,
            false => landing.fall_speed.max(down),
        };
        falling.push((entity, peak, ground.standing));

        if ground.standing && !landing.was_standing {
            let span = landing.full_speed - landing.min_speed;
            let strength = match span > 0.0 {
                true => ((landing.fall_speed - landing.min_speed) / span).clamp(0.0, 1.0),
                false => 1.0,
            };
            if strength > 0.0 {
                landed.push(Landed { entity, strength });
            }
        }
    }
    (landed, falling)
}

/// Writes back each character's peak and whether it is standing.
fn remember(resources: &mut Resources, falling: &[(Entity, f32, bool)]) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(landings) = registry.get_cpu_mut::<LandingImpulse>() else {
        return;
    };
    for (entity, peak, standing) in falling {
        if let Some(landing) = landings.get_mut(*entity) {
            landing.fall_speed = *peak;
            landing.was_standing = *standing;
        }
    }
}

/// Turns each landing into the impulse its source describes.
fn sourced(resources: &Resources, landed: &[Landed]) -> Vec<Impulse> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    landed
        .iter()
        .filter_map(|one| {
            let source = registry
                .get_cpu::<ImpulseSource>()
                .and_then(|sources| sources.get(one.entity))
                .copied()?;
            let at = registry
                .get_cpu::<kooch_ecs::hierarchy::GlobalTransform>()?
                .get(one.entity)
                .map(|global| global.matrix.to_scale_rotation_translation().2)?;
            Some(Impulse::from_source(source, at, one.strength))
        })
        .collect()
}

#[cfg(test)]
mod tests;
