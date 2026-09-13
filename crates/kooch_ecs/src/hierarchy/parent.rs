//! Parent component — marks an entity as a child of another.

use crate::component::Component;
use crate::entity::Entity;

/// Marks this entity as a child of another entity.
#[derive(Debug, Clone, Default, crate::Reflect)]
#[reflect(inspector = "read_only")]
pub struct Parent {
    pub entity: Entity,
}

impl Component for Parent {}

#[cfg(test)]
mod tests;
