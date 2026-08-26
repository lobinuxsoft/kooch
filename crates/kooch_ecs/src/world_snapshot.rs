//! [`WorldSnapshot`] — capture and restore a world *with its identities*.
//!
//! Saving through [`SceneDocument`](crate::scene::SceneDocument) and
//! loading it back is not a restore: the scene format keys entities by
//! name, so a round-trip despawns everything and respawns it. Handles
//! come back with different indices, different generations and a
//! different order, and the allocator's generation counters keep
//! climbing. Anything holding an [`Entity`] — a selection, a remote
//! mirror, a `Parent`, a system's cached handle — is silently pointing
//! at the wrong thing or at nothing.
//!
//! That is fine for loading a file authored elsewhere. It is wrong for
//! *stop*: pressing stop should leave the world as it was before play,
//! down to the identities, exactly as starting play never happened.
//!
//! [`WorldSnapshot`] therefore captures the [`EntityAllocator`] verbatim
//! alongside per-entity component values keyed by the concrete
//! [`Entity`], and restores both. Handles, generations, the free list
//! and the entity order all come back unchanged.
//!
//! # What is not covered
//!
//! Components with no reflector are not captured — the same limitation
//! the scene format has, since there is no generic way to copy an opaque
//! type. State a system keeps *outside* the ECS (in its own resource) is
//! likewise untouched: this snapshots the world, not the program.

use std::any::TypeId;

use kooch_core::resource::Resources;

use crate::allocator::EntityAllocator;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::dynamic_components::DynamicComponents;
use crate::entity::Entity;
use crate::reflect::ReflectValue;

/// One entity's reflected state, addressed by its exact handle.
struct EntityState {
    entity: Entity,
    /// Reflected components, as `(type, fields)`.
    components: Vec<(TypeId, Vec<(String, ReflectValue)>)>,
    /// Components parked by name because this binary has no type for
    /// them — carried so a host mirroring a foreign project restores
    /// those too.
    parked: Vec<(String, Vec<(String, ReflectValue)>)>,
}

/// A restorable capture of the world's entities and their components.
///
/// See the module docs for what it does and does not cover.
pub struct WorldSnapshot {
    /// The allocator as it stood, so restoring reinstates generations
    /// and the free list rather than continuing past them.
    allocator: EntityAllocator,
    /// Entity state in the world's own iteration order, which the
    /// restore reinstates — order is observable, both to systems and to
    /// a client reading the world.
    entities: Vec<EntityState>,
}

impl WorldSnapshot {
    /// Captures the current world.
    pub fn capture(resources: &Resources) -> Self {
        let allocator = resources
            .get::<EntityAllocator>()
            .cloned()
            .unwrap_or_default();

        let mut entities: Vec<EntityState> = Vec::new();
        if let (Some(archetypes), Some(registry)) = (
            resources.get::<ArchetypeRegistry>(),
            resources.get::<ComponentRegistry>(),
        ) {
            let dynamic = resources.get::<DynamicComponents>();
            for archetype in archetypes.iter_matching(&[]) {
                for &entity in archetype.entities() {
                    let components = archetype
                        .components()
                        .iter()
                        .filter_map(|tid| {
                            let fields = registry.reflect_get_fields(tid, entity)?;
                            Some((*tid, fields))
                        })
                        .collect();
                    let parked = dynamic
                        .as_ref()
                        .map(|d| {
                            d.iter_entity(entity)
                                .map(|(name, fields)| (name.to_owned(), fields.to_vec()))
                                .collect()
                        })
                        .unwrap_or_default();
                    entities.push(EntityState {
                        entity,
                        components,
                        parked,
                    });
                }
            }
        }
        Self {
            allocator,
            entities,
        }
    }

    /// Number of captured entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// `true` when nothing was captured.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Puts the captured world back.
    ///
    /// Clears whatever is there now, reinstates the allocator, then
    /// rebuilds each entity against its original handle — so an
    /// [`Entity`] taken before the capture still addresses the same
    /// thing afterwards.
    pub fn restore(&self, resources: &mut Resources) {
        clear_world(resources);

        if let Some(allocator) = resources.get_mut::<EntityAllocator>() {
            *allocator = self.allocator.clone();
            // The world was rebuilt underneath any GPU-side alive mask,
            // so every slot has to be re-synced, not just the ones that
            // happened to be dirty when the capture was taken.
            allocator.mark_all_pending_sync();
        }

        for state in &self.entities {
            for (type_id, fields) in &state.components {
                if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                    registry.insert_default_reflected(type_id, state.entity);
                    for (field, value) in fields {
                        let _ =
                            registry.reflect_set_field(type_id, state.entity, field, value.clone());
                    }
                }
                add_to_archetype(resources, state.entity, *type_id);
            }
            if !state.parked.is_empty() {
                if resources.get::<DynamicComponents>().is_none() {
                    resources.insert(DynamicComponents::new());
                }
                if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
                    for (name, fields) in &state.parked {
                        dynamic.insert(state.entity, name, fields.clone());
                    }
                }
            }
        }

        // Rebuilding walks each entity through a chain of archetypes, so
        // where it lands depends on the order its components went in —
        // not the order the world had. Put the observable order back.
        let order: Vec<Entity> = self.entities.iter().map(|e| e.entity).collect();
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.reorder_entities(&order);
        }
    }
}

/// Strips every entity's components and archetype membership.
fn clear_world(resources: &mut Resources) {
    let entities: Vec<Entity> = resources
        .get::<ArchetypeRegistry>()
        .map(|a| {
            a.iter_matching(&[])
                .flat_map(|arch| arch.entities().to_vec())
                .collect()
        })
        .unwrap_or_default();

    for entity in entities {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_entity(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
    }
    if let Some(dynamic) = resources.get_mut::<DynamicComponents>() {
        dynamic.clear();
    }
}

/// Moves `entity` into the archetype it belongs in after adding a type.
fn add_to_archetype(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    let current = match archetypes.entity_archetype(entity) {
        Some(current) => current,
        // First component: the entity is not in any archetype yet, so
        // start it from the empty one.
        None => {
            let empty = archetypes.get_or_create(Default::default());
            archetypes.register_entity(entity, empty);
            empty
        }
    };
    let next = archetypes.archetype_after_add_dynamic(current, type_id);
    archetypes.register_entity(entity, next);
}

#[cfg(test)]
mod tests;
