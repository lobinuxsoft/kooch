//! Internal command enum and helper aliases for the deferred command buffer.

use std::any::TypeId;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;
use crate::entity::Entity;

/// Type-erased function that inserts a component for an entity.
pub(super) type InsertFn =
    Box<dyn FnOnce(Entity, &mut ComponentRegistry, &mut ArchetypeRegistry) + Send + Sync>;

/// A single deferred ECS command.
pub(super) enum Command {
    /// Insert one or more components for an entity.
    InsertComponents {
        entity: Entity,
        inserts: Vec<InsertFn>,
    },
    /// Despawn an entity (mark dead, cleanup deferred to existing systems).
    Despawn(Entity),
    /// Remove a single component by TypeId.
    RemoveComponent { entity: Entity, type_id: TypeId },
}
