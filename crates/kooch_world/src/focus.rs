//! [`StreamingFocus`] — entities that drive chunk load/unload decisions.
//!
//! Any entity carrying `StreamingFocus` causes chunks within the LOD
//! ring radii (configured globally via [`super::lod::LodRingConfig`])
//! around its world position to load. Multiple focuses coexist (player
//! camera, AI factions, server-driven event regions) — the chunk
//! activation system computes the union of all active focus regions.
//!
//! Focuses use the entity's `GlobalTransform` (already relative to
//! `ActiveOrigin`), so they survive origin rebases without rewiring.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marker that an entity should drive chunk streaming. Pair with a
/// `GlobalTransform` (every spatial entity has one) — the activation
/// system reads the world position from there and triggers loads
/// within the LOD ring radii.
///
/// Why a component (not a resource): a single resource forces
/// "one focus only", which breaks the moment an event or AI needs to
/// be a streaming center independent of the player. As a component,
/// N focuses coexist trivially.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Streaming")]
pub struct StreamingFocus {
    /// When `false`, this focus is ignored by the activation system.
    /// Useful for paused / dormant entities that still exist in the
    /// world (sleeping NPCs, disabled event regions).
    pub active: bool,
    /// Higher value wins when the memory budget forces eviction
    /// choices between regions covered by different focuses. Convention:
    /// player camera = `0`, NPCs = `1-4`, server-driven event regions
    /// = `10+`.
    pub priority: u8,
}

impl Default for StreamingFocus {
    fn default() -> Self {
        Self {
            active: true,
            priority: 0,
        }
    }
}

impl Component for StreamingFocus {}

#[cfg(test)]
mod tests;
