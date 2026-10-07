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
use kooch_ecs::impulse::{Impulse, ImpulseSignal};
use kooch_ecs::reflect::FieldRange;
use kooch_physics::PhysicsWorld;
use kooch_physics::SolverBody;

use crate::grounded::Grounded;

/// Shakes listeners when this character lands, scaled by how fast it was falling.
///
/// 🔴 Carries its own signal. It took a separate `ImpulseSource` beside it, which stated nothing —
/// the two are always authored together — and made the common case three components (#1421).
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Impulse")]
pub struct LandingImpulse {
    /// Which signal: one of `kooch_ecs::impulse::shape`'s constants.
    #[reflect(choices = kooch_ecs::impulse::shape::SHAPE_CHOICES)]
    pub shape: u32,
    /// How far a listener is pushed, per axis, in metres at the source.
    pub amplitude: glam::Vec3,
    /// How long the whole signal lasts.
    #[reflect(range = DURATION_RANGE)]
    pub duration: f32,
    /// Inside this, full strength.
    #[reflect(range = DISTANCE_RANGE)]
    pub radius: f32,
    /// How much further it takes to fade to nothing past the radius.
    #[reflect(range = DISTANCE_RANGE)]
    pub dissipation: f32,
    /// Which listeners hear it. One sharing no bit is not shaken.
    #[reflect(layers)]
    pub channels: u32,
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
    /// Whether there was ground under the character last step, so a landing is an edge rather than
    /// a state.
    ///
    /// 🔴 Having GROUND, not standing on it. `Footing::stands()` is `Ground` only — a step is
    /// something you are getting over, in its own words — so walking a staircase flips `standing`
    /// false and true repeatedly and every flip read as a landing. The scene this was smoke-tested
    /// in is built out of steps and ramps, so the camera never stopped shaking (#1421).
    #[reflect(skip)]
    pub was_supported: bool,
}

const SPEED_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

const DURATION_RANGE: FieldRange = FieldRange {
    min: 0.01,
    max: 10.0,
    step: 0.01,
};

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1000.0,
    step: 0.5,
};

impl LandingImpulse {
    /// 🔴 The fields are flat because reflection takes primitives only, so the Inspector shows them
    /// without a level of nesting — which is the better reading anyway. This is where they become
    /// the signal the bus carries.
    fn signal(&self) -> ImpulseSignal {
        ImpulseSignal {
            shape: self.shape,
            amplitude: self.amplitude,
            duration: self.duration,
            radius: self.radius,
            dissipation: self.dissipation,
            channels: self.channels,
        }
    }
}

impl Default for LandingImpulse {
    fn default() -> Self {
        Self {
            // Stepping off a kerb is about 2 m/s; a jump lands around 7.
            min_speed: 3.0,
            full_speed: 15.0,
            shape: kooch_ecs::impulse::shape::BUMP,
            amplitude: glam::Vec3::new(0.0, 0.3, 0.0),
            duration: 0.2,
            radius: 5.0,
            dissipation: 20.0,
            channels: u32::MAX,
            fall_speed: 0.0,
            was_supported: true,
        }
    }
}

impl Component for LandingImpulse {}

/// Whether there is ground under the character at all.
///
/// 🔴 Not whether it can STAND on it. `Footing::stands()` is `Ground` only — a step is something
/// you are getting over, in its own words — so a staircase flips `standing` false and true
/// repeatedly, and reading that field made every flip a landing (#1421). A step or a ramp has a
/// normal; only the air has none.
fn supported(ground: &Grounded) -> bool {
    ground.normal.length_squared() > 1e-6
}

/// What one character's landing is worth, planned before anything is written.
struct Landed {
    entity: Entity,
    strength: f32,
    signal: ImpulseSignal,
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
        let supported = supported(ground);
        let down = bodies
            .get(entity)
            .and_then(|body| world.handle(body.slot()))
            .and_then(|handle| world.backend().linear_velocity(handle))
            // 🔴 Against the surface's own normal, not world down: landing on the inside of a
            // sphere is still landing, and on a planet "down" is wherever you are standing.
            .map(|velocity| -velocity.dot(ground.normal.normalize_or(glam::Vec3::Y)))
            .unwrap_or(0.0);

        let peak = match supported {
            // With ground under it the record resets, so the next fall starts from nothing.
            true => 0.0,
            false => landing.fall_speed.max(down),
        };
        falling.push((entity, peak, supported));

        if supported && !landing.was_supported {
            let span = landing.full_speed - landing.min_speed;
            let strength = match span > 0.0 {
                true => ((landing.fall_speed - landing.min_speed) / span).clamp(0.0, 1.0),
                false => 1.0,
            };
            if strength > 0.0 {
                landed.push(Landed {
                    entity,
                    strength,
                    signal: landing.signal(),
                });
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
    for (entity, peak, supported) in falling {
        if let Some(landing) = landings.get_mut(*entity) {
            landing.fall_speed = *peak;
            landing.was_supported = *supported;
        }
    }
}

/// Turns each landing into the impulse its trigger describes, from where the character is.
fn sourced(resources: &Resources, landed: &[Landed]) -> Vec<Impulse> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    landed
        .iter()
        .filter_map(|one| {
            let at = registry
                .get_cpu::<kooch_ecs::hierarchy::GlobalTransform>()?
                .get(one.entity)
                .map(|global| global.matrix.to_scale_rotation_translation().2)?;
            Some(Impulse::new(one.signal, at, one.strength))
        })
        .collect()
}

#[cfg(test)]
mod tests;
