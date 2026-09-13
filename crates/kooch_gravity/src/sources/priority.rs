//! [`GravityPriority`] — the zone that overrules the planet.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Which sources this field overrules: a higher `level` suppresses lower ones in proportion to its
/// reach at a point; equal levels sum. Absent is level 0.
/// It follows the overriding source's falloff, so give zones a soft edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
#[reflect(category = "Physics")]
pub struct GravityPriority {
    /// Higher overrules lower. Equal levels sum.
    pub level: i32,
}

impl Component for GravityPriority {}
