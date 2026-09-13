//! [`Facing`] — which way a character is being steered.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Where gameplay steers a character, in world space, written **every frame**: direction turns the
/// body, length is the throttle.
/// 🔴 Zero means stop — skipping the write left a character walking on its own.
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct Facing {
    /// Direction and throttle in one: flattened against the local up,
    /// and its length clamped to `1` before it scales the top speed.
    pub direction: Vec3,
}

impl Component for Facing {}
