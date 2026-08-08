//! Parent component — marks an entity as a child of another.

use crate::component::Component;
use crate::entity::Entity;

/// Marks this entity as a child of another entity.
///
/// `Parent` is the authoritative side of the relationship — the hierarchy
/// sync system updates `Children` to match.
///
/// # An ordinary component
///
/// This used to be special. Reflection had no way to express "points at an
/// entity", so `Parent` reflected its target as a `"index:generation"`
/// string that nothing could resolve, and the scene format carried the
/// real link out of band in `EntityDescription::parent_index`.
///
/// With [`FieldKind::EntityRef`](crate::reflect::FieldKind::EntityRef) the
/// link is just a field, saved and resolved by the same generic pass that
/// handles every other component holding an entity. Read-only in the
/// inspector because reparenting goes through the World panel, which
/// maintains `Children` alongside it.
#[derive(Debug, Clone, Default, crate::Reflect)]
#[reflect(inspector = "read_only")]
pub struct Parent {
    pub entity: Entity,
}

impl Component for Parent {}

#[cfg(test)]
mod tests;
