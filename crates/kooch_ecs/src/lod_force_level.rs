//! `LodForceLevel` component — overrides the meshlet pipeline's LOD selector for this entity,
//! forcing all visible meshlets to come from a specific chain depth.

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Forces the meshlet cull to emit only meshlets at the specified chain depth for this instance.
/// `0` = LOD 0 (full detail), each integer up the chain corresponds to one simplification step.
#[derive(Debug, Clone, Copy, Default, Reflect)]
#[reflect(category = "Debug")]
pub struct LodForceLevel {
    /// Chain depth to render. Out-of-range values (above the chain's max depth for this mesh)
    /// silently produce zero visible meshlets — the cull still runs but every thread fails the
    /// `lod_level == lod_force_level` test.
    pub level: u32,
}

impl Component for LodForceLevel {}

#[cfg(test)]
mod tests;
