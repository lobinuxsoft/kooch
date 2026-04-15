//! Dynamic rigid body component.
//!
//! Marks an entity as simulated by the physics engine: mass, drag and
//! gravity participation. Complements [`CollisionShape`] (shape) with
//! behaviour (how it reacts to forces).

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Dynamic body — fully simulated by the physics engine.
///
/// # Default
///
/// - `mass`: 1.0 kg
/// - `linear_drag`: 0.0
/// - `angular_drag`: 0.05
/// - `use_gravity`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Physics")]
pub struct DynamicBody {
    /// Mass in kilograms.
    pub mass: f32,
    /// Linear drag coefficient.
    pub linear_drag: f32,
    /// Angular drag coefficient.
    pub angular_drag: f32,
    /// Whether gravity affects this body.
    pub use_gravity: bool,
}

impl Default for DynamicBody {
    fn default() -> Self {
        Self {
            mass: 1.0,
            linear_drag: 0.0,
            angular_drag: 0.05,
            use_gravity: true,
        }
    }
}

impl Component for DynamicBody {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let b = DynamicBody::default();
        assert_eq!(b.mass, 1.0);
        assert_eq!(b.linear_drag, 0.0);
        assert_eq!(b.angular_drag, 0.05);
        assert!(b.use_gravity);
    }

    #[test]
    fn reflect_fields() {
        let b = DynamicBody::default();
        let fields = b.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["mass", "linear_drag", "angular_drag", "use_gravity"]);
    }
}
