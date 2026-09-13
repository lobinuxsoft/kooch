//! [`StreamingFocus`]: entities whose position loads the chunks within the LOD ring radii
//! ([`super::lod::LodRingConfig`]); many coexist and activation takes their union.
//! Reads `GlobalTransform`, so a focus survives origin rebases.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Marks an entity as a streaming centre. A component, not a resource, because a single resource
/// allows one focus and AI or events need their own.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Streaming")]
pub struct StreamingFocus {
    /// When `false`, this focus is ignored by the activation system.
    /// Useful for paused / dormant entities that still exist in the
    /// world (sleeping NPCs, disabled event regions).
    pub active: bool,
    /// Higher wins when the budget forces eviction. Convention: player camera 0, NPCs 1–4, server
    /// events 10+.
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
