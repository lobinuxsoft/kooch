//! Static rigid body component.
//!
//! Marks an entity as part of the immovable world: never moves, never
//! reacts to forces. Dynamic bodies collide against it as infinite mass.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Static body — fixed, never moves.
///
/// Unit struct — its mere presence tags the entity as static.
#[derive(Debug, Clone, Copy, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct StaticBody;

impl Component for StaticBody {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let _ = StaticBody;
    }

    #[test]
    fn reflect_fields_empty() {
        let b = StaticBody;
        assert!(b.reflect_fields().is_empty());
    }
}
