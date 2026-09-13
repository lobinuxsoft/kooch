//! [`GlobalGravity`] — a uniform field with no source and no falloff.

use glam::Vec3;

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// A uniform field with no source and no falloff — the default down, as a component a scene can
/// author, move and switch off. Earth, downward.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct GlobalGravity {
    /// Acceleration in metres per second squared, in world space.
    pub acceleration: Vec3,
}

impl Default for GlobalGravity {
    fn default() -> Self {
        Self {
            acceleration: Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl Component for GlobalGravity {}
