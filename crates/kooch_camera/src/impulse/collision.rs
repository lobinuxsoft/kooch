//! An impulse the world fires by itself, from a contact (#1255).
//!
//! Cinemachine's `CinemachineCollisionImpulseSource`. Without it an impulse needs a line of game
//! code to fire, and the one case every game wants — a landing — is a contact.
//!
//! 🔴 Scaled by the force, so a drop from a step and a drop from a roof are not the same shake. The
//! scale is the force at which the authored amplitude is reached in full; past it the signal is
//! clamped, because a fall off the map should not throw the camera into the next county.

use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;
use kooch_physics::plugin::ContactForce;

use kooch_ecs::impulse::{Impulse, ImpulseSignal};

/// Fires this entity's [`ImpulseSource`] when it is hit hard enough.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Impulse")]
pub struct CollisionImpulse {
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
    /// Which listeners hear it.
    #[reflect(layers)]
    pub channels: u32,
    /// Below this, nothing. Keeps a body resting on the floor from shaking the camera forever.
    #[reflect(range = FORCE_RANGE)]
    pub min_force: f32,
    /// The force at which the source's amplitude is felt in full. Softer hits scale down, harder
    /// ones are clamped here.
    #[reflect(range = FORCE_RANGE)]
    pub full_force: f32,
}

const FORCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100_000.0,
    step: 10.0,
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

impl CollisionImpulse {
    /// The flat fields as the signal the bus carries — reflection takes primitives only.
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

impl Default for CollisionImpulse {
    fn default() -> Self {
        Self {
            shape: kooch_ecs::impulse::shape::BUMP,
            amplitude: glam::Vec3::new(0.0, 0.3, 0.0),
            duration: 0.2,
            radius: 5.0,
            dissipation: 20.0,
            channels: u32::MAX,
            // A body at rest reports a contact force equal to its weight, so the floor is the thing
            // being filtered out here.
            min_force: 500.0,
            full_force: 5_000.0,
        }
    }
}

impl Component for CollisionImpulse {}

/// Turns contacts into impulses, published for anything to hear.
pub fn impulses_from_contacts(resources: &mut Resources) {
    let fired = planned(resources);
    if fired.is_empty() {
        return;
    }
    if let Some(events) = resources.get_mut::<kooch_core::event::Events<Impulse>>() {
        for impulse in fired {
            events.send(impulse);
        }
    }
}

/// What each contact asks for, read before anything is emitted.
fn planned(resources: &Resources) -> Vec<Impulse> {
    let Some(events) = resources.get::<kooch_core::event::Events<ContactForce>>() else {
        return Vec::new();
    };
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let mut fired = Vec::new();
    // 🔴 The FIXED list. A contact is published by the solver on its own cadence, and reading the
    // frame list would lose five arrivals in six at 360 fps — what killed every post-process volume
    // in a build (#1312).
    for contact in events.read_fixed() {
        for entity in [contact.a, contact.b] {
            let Some(collision) = registry
                .get_cpu::<CollisionImpulse>()
                .and_then(|storage| storage.get(entity))
            else {
                continue;
            };
            if contact.max_force_magnitude < collision.min_force {
                continue;
            }
            let Some(at) = where_it_is(registry, entity) else {
                continue;
            };
            fired.push(Impulse::new(
                collision.signal(),
                at,
                strength(contact.max_force_magnitude, *collision),
            ));
        }
    }
    fired
}

/// How much of the source's amplitude a hit of `force` is worth, clamped at `full_force`.
fn strength(force: f32, collision: CollisionImpulse) -> f32 {
    let span = collision.full_force - collision.min_force;
    match span > 0.0 {
        true => ((force - collision.min_force) / span).clamp(0.0, 1.0),
        // A zero span is "anything over the floor is a full hit", which is a legitimate setting
        // rather than a division to guard against.
        false => 1.0,
    }
}

fn where_it_is(registry: &ComponentRegistry, entity: Entity) -> Option<glam::Vec3> {
    registry
        .get_cpu::<kooch_ecs::hierarchy::GlobalTransform>()?
        .get(entity)
        .map(|global| global.matrix.to_scale_rotation_translation().2)
}

#[cfg(test)]
mod tests;
