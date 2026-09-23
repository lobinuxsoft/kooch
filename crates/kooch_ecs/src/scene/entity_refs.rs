//! Turning entity references between their live and saved forms.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::resource::Resources;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::persistent_id::{EntityGuid, PersistentId, PersistentIdAllocator};
use crate::reflect::{EntityRef, FieldKind, ReflectValue};
use crate::scene_member::SceneMember;

/// Every entity referenced by a reflected field anywhere in the world.
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

    // Every id alive, not only the referenced ones: an entity nobody points at still owns its id,
    // and an allocator that never heard of it hands the same one out again next session.
    let alive: Vec<EntityGuid> = resources
        .get::<ComponentRegistry>()
        .and_then(|components| components.get_cpu::<PersistentId>())
        .map(|storage| storage.iter().map(|(_, p)| p.id).collect())
        .unwrap_or_default();
    if let Some(allocator) = resources.get_mut::<PersistentIdAllocator>() {
        for id in alive {
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

/// Takes the ids a load brought in: the allocator learns every one of them, and an id two entities
/// in the same scene claim is repaired here, deterministically.
///
/// 🔴 A duplicate id is not cosmetic: every reference to it resolves by id, so a child whose
/// `Parent` names one lands under whichever entity a `HashMap` happened to yield — a different
/// parent per machine and per run.
pub(super) fn adopt_ids(resources: &mut Resources) {
    let claims: Vec<(Guid, EntityGuid, Entity)> = {
        let Some(components) = resources.get::<ComponentRegistry>() else {
            return;
        };
        let (Some(ids), Some(members)) = (
            components.get_cpu::<PersistentId>(),
            components.get_cpu::<SceneMember>(),
        ) else {
            return;
        };
        let mut claims: Vec<(Guid, EntityGuid, Entity)> = ids
            .iter()
            .filter_map(|(&entity, persistent)| {
                Some((members.get(entity)?.scene, persistent.id, entity))
            })
            .collect();
        // The winner of a clash is the lowest entity index, not the first the storage yields:
        // storage order is a hash, and a repair that varies by run is the bug again.
        claims.sort_by_key(|&(scene, id, entity)| (scene.to_string(), id.get(), entity.index()));
        claims
    };

    if resources.get::<PersistentIdAllocator>().is_none() {
        resources.insert(PersistentIdAllocator::new());
    }
    if let Some(allocator) = resources.get_mut::<PersistentIdAllocator>() {
        for &(_, id, _) in &claims {
            allocator.observe(id);
        }
    }

    let clashing: Vec<(Entity, EntityGuid)> = claims
        .windows(2)
        .filter(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
        .map(|pair| (pair[1].2, pair[1].1))
        .collect();
    if clashing.is_empty() {
        return;
    }

    let Some(allocator) = resources.get_mut::<PersistentIdAllocator>() else {
        return;
    };
    let repaired: Vec<(Entity, EntityGuid)> = clashing
        .iter()
        .map(|&(entity, _)| (entity, allocator.allocate()))
        .collect();

    for (&(entity, was), &(_, now)) in clashing.iter().zip(repaired.iter()) {
        tracing::error!(
            target: "kooch_ecs::scene",
            %entity,
            %was,
            %now,
            "two entities in one scene claimed the same identity; the later one was given a new \
             id. Every reference to it pointed at the other entity — save the scene to keep the \
             repair",
        );
    }

    if let Some(components) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = components.get_cpu_mut::<PersistentId>()
    {
        for &(entity, id) in &repaired {
            storage.insert(entity, PersistentId::new(id));
        }
    }
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
pub(super) fn resolve_deferred(resources: &mut Resources, deferred: Vec<DeferredRef>) {
    if deferred.is_empty() {
        return;
    }

    // Ids arrived as ordinary `PersistentId` components during the spawn pass, so the map is
    // everything that ended up carrying one.
    let by_id: HashMap<(Guid, EntityGuid), Entity> = resources
        .get::<ComponentRegistry>()
        .map(|components| {
            let members = components.get_cpu::<SceneMember>();
            components
                .get_cpu::<PersistentId>()
                .map(|storage| {
                    let mut by_id: HashMap<(Guid, EntityGuid), Entity> = HashMap::new();
                    for (&entity, persistent) in storage.iter() {
                        let Some(scene) = members.and_then(|m| m.get(entity)).map(|m| m.scene)
                        else {
                            continue;
                        };
                        // Lowest index wins, as `adopt_ids` repairs: a clash that slipped through
                        // must still resolve the same way twice.
                        by_id
                            .entry((scene, persistent.id))
                            .and_modify(|held| {
                                if entity.index() < held.index() {
                                    *held = entity;
                                }
                            })
                            .or_insert(entity);
                    }
                    by_id
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    // Which scene each referring entity lives in, so a reference that does
    // not name a scene resolves within its own.
    let scene_of = |resources: &Resources, entity: Entity| -> Option<Guid> {
        resources
            .get::<ComponentRegistry>()?
            .get_cpu::<SceneMember>()?
            .get(entity)
            .map(|member| member.scene)
    };

    for deferred in deferred {
        let Some(id) = deferred.reference.persistent_id() else {
            continue;
        };

        // A reference naming no scene means "my own".
        let Some(scene) = deferred
            .reference
            .scene()
            .or_else(|| scene_of(resources, deferred.entity))
        else {
            tracing::debug!(
                target: "kooch_ecs::scene",
                field = %deferred.field,
                "reference held by an entity belonging to no scene; left unresolved",
            );
            continue;
        };

        // A reference from disk names a **file**; the world is keyed by **instance**. When exactly
        // one copy of that file is open, that is the one it means.
        let scene = match by_id.contains_key(&(scene, id)) {
            true => scene,
            false => match instances_of(resources, scene).as_slice() {
                [only] => *only,
                [] => scene,
                several => {
                    tracing::warn!(
                        target: "kooch_ecs::scene",
                        field = %deferred.field,
                        copies = several.len(),
                        "reference names a scene that is open more than once; \
                         nothing in it says which copy, so it is left unset",
                    );
                    scene
                }
            },
        };

        let resolved = match by_id.get(&(scene, id)) {
            Some(&target) => ReflectValue::EntityRef(Some(EntityRef::live(target))),
            None => {
                // Ordinary when the target's scene is not open — a cross-scene reference resolves
                // once both are loaded, and under world cells (#566) a non-resident target is the
                // normal case rather than a broken file.
                tracing::debug!(
                    target: "kooch_ecs::scene",
                    field = %deferred.field,
                    %id,
                    "reference target is not loaded; left unset",
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
                target: "kooch_ecs::scene",
                field = %deferred.field,
                %error,
                "failed to write a resolved entity reference",
            );
        }
    }
}

/// Rewrites a field value for storage, turning a live reference into a persistent one.
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
                target: "kooch_ecs::scene",
                entity = target.index(),
                "reference target has no persistent id; saved as unset",
            );
            ReflectValue::EntityRef(None)
        }
    }
}

/// The instance ids of every open copy of the file `source`.
fn instances_of(resources: &Resources, source: Guid) -> Vec<Guid> {
    resources
        .get::<crate::scene_manager::SceneManager>()
        .map(|manager| manager.instances_of(source).map(|scene| scene.id).collect())
        .unwrap_or_default()
}
