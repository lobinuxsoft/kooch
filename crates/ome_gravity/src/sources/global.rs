//! [`GlobalGravity`] — a uniform field with no source and no falloff.

use glam::Vec3;

use ome_ecs::Reflect;
use ome_ecs::component::Component;

/// A uniform field with no source and no falloff.
///
/// What every scene has by default, expressed as a component so it can be
/// authored, moved between scenes, and switched off — rather than living
/// only in the plugin's configuration where a level cannot reach it.
///
/// # Default
///
/// Earth, downward.
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
