//! Kinematic rigid body component.
//!
//! Marks an entity as user-driven: the physics engine does not apply
//! forces, but it still participates in collision detection. Position
//! is updated by gameplay code or animation.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Kinematic body — moved by user code, not by physics forces.
///
/// # Default
///
/// - `push_force`: 0.0 (does not push dynamic bodies on contact)
#[derive(Debug, Clone, Copy, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct KinematicBody {
    /// Force applied to dynamic bodies on contact. `0.0` means no push.
    pub push_force: f32,
}

impl Component for KinematicBody {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let b = KinematicBody::default();
        assert_eq!(b.push_force, 0.0);
    }

    #[test]
    fn reflect_fields() {
        let b = KinematicBody::default();
        let fields = b.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["push_force"]);
    }
}
