//! Velocity component — read-only output of the physics engine.
//!
//! Present on dynamic and kinematic bodies to expose their current
//! linear and angular velocity to gameplay code and the inspector.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Linear and angular velocity.
///
/// Read-only from the inspector's point of view — the physics engine
/// writes these fields every simulation step.
///
/// # Default
///
/// - `linear`: zero
/// - `angular`: zero
#[derive(Debug, Clone, Copy, Default, Reflect)]
#[reflect(inspector = "read_only", category = "Physics")]
pub struct Velocity {
    /// Linear velocity in world space (units per second).
    pub linear: Vec3,
    /// Angular velocity in world space (radians per second).
    pub angular: Vec3,
}

impl Component for Velocity {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::{InspectorVisibility, Reflect};

    #[test]
    fn default_values() {
        let v = Velocity::default();
        assert_eq!(v.linear, Vec3::ZERO);
        assert_eq!(v.angular, Vec3::ZERO);
    }

    #[test]
    fn reflect_fields() {
        let v = Velocity::default();
        let fields = v.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["linear", "angular"]);
    }

    #[test]
    fn visibility_is_read_only() {
        assert_eq!(Velocity::inspector_visibility(), InspectorVisibility::ReadOnly);
    }
}
