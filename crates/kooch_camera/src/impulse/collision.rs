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

use super::{ImpulseSource, Impulses};

/// Fires this entity's [`ImpulseSource`] when it is hit hard enough.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CollisionImpulse {
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

impl Default for CollisionImpulse {
    fn default() -> Self {
        Self {
            // A body at rest reports a contact force equal to its weight, so the floor is the thing
            // being filtered out here.
            min_force: 500.0,
            full_force: 5_000.0,
        }
    }
}

impl Component for CollisionImpulse {}

/// Turns contacts into impulses.
pub fn impulses_from_contacts(resources: &mut Resources) {
    let fired = planned(resources);
    if fired.is_empty() {
        return;
    }
    let Some(mut impulses) = resources.remove::<Impulses>() else {
        return;
    };
    for (source, at) in fired {
        impulses.emit(source, at);
    }
    resources.insert(impulses);
}

/// What each contact asks for, read before anything is emitted.
fn planned(resources: &Resources) -> Vec<(ImpulseSource, glam::Vec3)> {
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
            let Some(source) = registry
                .get_cpu::<ImpulseSource>()
                .and_then(|storage| storage.get(entity))
                .copied()
            else {
                continue;
            };
            let Some(at) = where_it_is(registry, entity) else {
                continue;
            };
            fired.push((scaled(source, contact.max_force_magnitude, *collision), at));
        }
    }
    fired
}

/// The source's amplitude scaled by how hard the hit was, clamped at `full_force`.
fn scaled(mut source: ImpulseSource, force: f32, collision: CollisionImpulse) -> ImpulseSource {
    let span = collision.full_force - collision.min_force;
    let strength = match span > 0.0 {
        true => ((force - collision.min_force) / span).clamp(0.0, 1.0),
        // A zero span is "anything over the floor is a full hit", which is a legitimate setting
        // rather than a division to guard against.
        false => 1.0,
    };
    source.amplitude *= strength;
    source
}

fn where_it_is(registry: &ComponentRegistry, entity: Entity) -> Option<glam::Vec3> {
    registry
        .get_cpu::<kooch_ecs::hierarchy::GlobalTransform>()?
        .get(entity)
        .map(|global| global.matrix.to_scale_rotation_translation().2)
}

#[cfg(test)]
mod tests;
