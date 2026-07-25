//! Turning entity references between their live and saved forms.
//!
//! A component in memory holds [`EntityRef::Live`]; a file holds
//! [`EntityRef::Persistent`]. This module owns both directions, so the
//! conversion lives in one place instead of being re-derived per component
//! the way `parent_index` was.
//!
//! # Why saving assigns identity
//!
//! [`PersistentId`] is opt-in — only entities something points at carry
//! one. "Something points at it" is not known until a reference is
//! written, so the save path discovers it: [`assign_ids_to_referenced`]
//! walks every reflected field, finds the entities being referenced, and
//! gives each one an id if it has none.
//!
//! That is why saving a scene mutates the world. The alternative is
//! either an id on every entity in a galaxy, or asking authors to tick a
//! box before a reference will survive a reload.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use ome_core::resource::Resources;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::persistent_id::{EntityGuid, PersistentId, PersistentIdAllocator};
use crate::reflect::{EntityRef, FieldKind, ReflectValue};

/// Every entity referenced by a reflected field anywhere in the world.
///
/// Reads only components that declare an [`FieldKind::EntityRef`] field —
/// the check is static metadata, so components without references cost
/// nothing beyond the lookup.
fn referenced_entities(resources: &Resources) -> HashSet<Entity> {
    let mut referenced = HashSet::new();

    let (Some(archetypes), Some(components)) = (
        resources.get::<ArchetypeRegistry>(),
        resources.get::<ComponentRegistry>(),
    ) else {
        return referenced;
    };

    for archetype in archetypes.iter_matching(&[]) {
        // Which components in this archetype can hold a reference at all.
        let referencing: Vec<TypeId> = archetype
            .components()
            .iter()
            .copied()
            .filter(|type_id| {
                components
                    .reflect_field_metas(type_id)
                    .is_some_and(|metas| metas.iter().any(|m| m.kind == FieldKind::EntityRef))
            })
            .collect();

        if referencing.is_empty() {
            continue;
        }

        for &entity in archetype.entities() {
            for type_id in &referencing {
                let Some(fields) = components.reflect_get_fields(type_id, entity) else {
                    continue;
                };
                for (_, value) in fields {
                    if let ReflectValue::EntityRef(Some(reference)) = value
                        && let Some(target) = reference.entity()
                    {
                        referenced.insert(target);
                    }
                }
            }
        }
    }

    referenced
}

/// Gives a [`PersistentId`] to every entity something references.
///
/// Returns the resulting `Entity -> EntityGuid` map, which the save path
/// then uses to resolve references without looking each one up again.
///
/// Entities that already carry an id keep it — reassigning would break
/// every reference in every *other* scene that points here.
pub(super) fn assign_ids_to_referenced(resources: &mut Resources) -> HashMap<Entity, EntityGuid> {
    let referenced = referenced_entities(resources);
    let mut ids = HashMap::with_capacity(referenced.len());
    if referenced.is_empty() {
        return ids;
    }

    // The allocator is created on demand: a hand-built `Resources` (tests,
    // headless tools) will not have had `EcsPlugin` insert one.
    if resources.get::<PersistentIdAllocator>().is_none() {
        resources.insert(PersistentIdAllocator::new());
    }

    // Existing ids first, so the allocator never reissues one that a file
    // somewhere already refers to.
    let mut missing = Vec::new();
    if let Some(components) = resources.get::<ComponentRegistry>() {
        let existing = components.get_cpu::<PersistentId>();
        for &entity in &referenced {
            match existing.and_then(|storage| storage.get(entity)) {
                Some(persistent) => {
                    ids.insert(entity, persistent.id);
                }
                None => missing.push(entity),
            }
        }
    }

    if let Some(allocator) = resources.get_mut::<PersistentIdAllocator>() {
        for &id in ids.values() {
            allocator.observe(id);
        }
    }

    // Allocate outside the loop above: `observe` must have seen every
    // existing id before the first fresh one is handed out.
    let fresh: Vec<(Entity, EntityGuid)> = {
        let Some(allocator) = resources.get_mut::<PersistentIdAllocator>() else {
            return ids;
        };
        missing
            .iter()
            .map(|&entity| (entity, allocator.allocate()))
            .collect()
    };

    if let Some(components) = resources.get_mut::<ComponentRegistry>() {
        components.register_cpu_reflected::<PersistentId>();
        if let Some(storage) = components.get_cpu_mut::<PersistentId>() {
            for &(entity, id) in &fresh {
                storage.insert(entity, PersistentId::new(id));
            }
        }
    }

    // The archetype has to learn about the new component or nothing will
    // find it — including the save walk that is about to run.
    let persistent_tid = TypeId::of::<PersistentId>();
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        for &(entity, _) in &fresh {
            if let Some(current) = archetypes.entity_archetype(entity) {
                let next = archetypes.archetype_after_add_dynamic(current, persistent_tid);
                archetypes.register_entity(entity, next);
            }
        }
    }

    ids.extend(fresh);
    ids
}

/// A reference the spawn pass could not write yet, kept until the
/// entities it points at exist.
pub(super) struct DeferredRef {
    pub entity: Entity,
    pub type_id: TypeId,
    pub field: String,
    pub reference: EntityRef,
}

/// Resolves every deferred reference against the entities that now exist.
///
/// This is the half that makes references generic. `parent_index` worked
/// because `Parent` was special-cased in both directions; anything else
/// holding an entity — a joint, a trigger's target, a camera's follow
/// subject — had nowhere to be resolved. Here the field's *kind* decides,
/// so a component the engine has never heard of resolves the same way.
///
/// A reference whose target is missing is left unset rather than failing
/// the load. Under world-cell streaming that is the normal state for a
/// reference into a cell that is not resident; failing here would make an
/// ordinary streaming boundary look like a corrupt scene.
pub(super) fn resolve_deferred(resources: &mut Resources, deferred: Vec<DeferredRef>) {
    if deferred.is_empty() {
        return;
    }

    // Ids arrived as ordinary `PersistentId` components during the spawn
    // pass, so the map is simply everything that ended up carrying one.
    let by_id: HashMap<EntityGuid, Entity> = resources
        .get::<ComponentRegistry>()
        .and_then(|components| components.get_cpu::<PersistentId>())
        .map(|storage| {
            storage
                .iter()
                .map(|(&entity, persistent)| (persistent.id, entity))
                .collect()
        })
        .unwrap_or_default();

    for deferred in deferred {
        let Some(id) = deferred.reference.persistent_id() else {
            continue;
        };

        // A cross-scene reference stays unresolved until that scene is
        // loaded. Not an error — see the doc comment.
        if deferred.reference.scene().is_some() {
            tracing::debug!(
                target: "ome_ecs::scene",
                field = %deferred.field,
                "cross-scene reference left unresolved; its scene is not loaded",
            );
            continue;
        }

        let resolved = match by_id.get(&id) {
            Some(&target) => ReflectValue::EntityRef(Some(EntityRef::live(target))),
            None => {
                tracing::warn!(
                    target: "ome_ecs::scene",
                    field = %deferred.field,
                    %id,
                    "reference points at an entity this scene does not contain",
                );
                ReflectValue::EntityRef(None)
            }
        };

        if let Some(components) = resources.get_mut::<ComponentRegistry>()
            && let Err(error) = components.reflect_set_field(
                &deferred.type_id,
                deferred.entity,
                &deferred.field,
                resolved,
            )
        {
            tracing::warn!(
                target: "ome_ecs::scene",
                field = %deferred.field,
                %error,
                "failed to write a resolved entity reference",
            );
        }
    }
}

/// Rewrites a field value for storage, turning a live reference into a
/// persistent one.
///
/// A reference whose target has no id resolves to `None` rather than
/// failing the save: the target is an entity that was despawned, or an
/// ephemeral one the scene deliberately excludes. Writing "points at
/// nothing" is honest; keeping the handle would not survive the reload
/// anyway, and refusing to save would lose the rest of the scene too.
pub(super) fn to_persistent(
    value: ReflectValue,
    ids: &HashMap<Entity, EntityGuid>,
) -> ReflectValue {
    let ReflectValue::EntityRef(Some(reference)) = value else {
        return value;
    };
    let Some(target) = reference.entity() else {
        // Already persistent — a value that came back from a file
        // untouched, such as a parked dynamic component.
        return ReflectValue::EntityRef(Some(reference));
    };

    match ids.get(&target) {
        Some(&id) => ReflectValue::EntityRef(Some(EntityRef::same_scene(id))),
        None => {
            tracing::debug!(
                target: "ome_ecs::scene",
                entity = target.index(),
                "reference target has no persistent id; saved as unset",
            );
            ReflectValue::EntityRef(None)
        }
    }
}
