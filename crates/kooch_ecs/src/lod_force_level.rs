//! `LodForceLevel` component — overrides the meshlet pipeline's LOD
//! selector for this entity, forcing all visible meshlets to come
//! from a specific chain depth.
//!
//! Used by the editor's side-by-side LOD inspector (#467): when an
//! artist clicks "Show LOD stack" on an entity that owns a
//! [`MeshRenderer`](crate::mesh_renderer::MeshRenderer), the editor
//! spawns N ghost copies of that entity with different
//! `Translation`s and `LodForceLevel(0..N)` so each chain layer
//! renders in isolation. Comparing cluster shapes between layers
//! tells the artist whether adjacent levels tile cleanly or pile
//! geometry on top of each other (Z-fight diagnosis).

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Forces the meshlet cull to emit only meshlets at the specified
/// chain depth for this instance. `0` = LOD 0 (full detail), each
/// integer up the chain corresponds to one simplification step.
///
/// When this component is absent the entity's meshlet pipeline runs
/// the normal screen-space-error LOD selector.
#[derive(Debug, Clone, Copy, Default, Reflect)]
#[reflect(category = "Debug")]
pub struct LodForceLevel {
    /// Chain depth to render. Out-of-range values (above the chain's
    /// max depth for this mesh) silently produce zero visible
    /// meshlets — the cull still runs but every thread fails the
    /// `lod_level == lod_force_level` test.
    pub level: u32,
}

impl Component for LodForceLevel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let l = LodForceLevel::default();
        assert_eq!(l.level, 0);
    }

    #[test]
    fn reflect_field_exposed() {
        let l = LodForceLevel { level: 3 };
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(names, &["level"]);
    }
}
