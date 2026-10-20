//! Moving an entity between archetypes: the one way to do it (#891).
//!
//! 🔴 Both halves or neither. An archetype changed without the row moving leaves that entity
//! pointing at the table it used to be in, where it reads **another entity's components** — no
//! error, no warning. That is why every site that changes an archetype goes through here, and why
//! this is the only place that calls [`ComponentRegistry::settle`].

use kooch_core::resource::Resources;

use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::{Component, ComponentRegistry, Settled, StorageId};
use crate::entity::Entity;

/// Moves `entity` into `archetype`: its values first, then the archetype it is filed under.
pub fn move_entity(resources: &mut Resources, entity: Entity, archetype: ArchetypeId) {
    let Some(mut components) = resources.remove::<ComponentRegistry>() else {
        return;
    };
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        move_in(&mut components, archetypes, entity, archetype);
    }
    resources.insert(components);
}

/// Files `entity` under the empty archetype, unless it is already filed somewhere.
///
/// The start of an entity's life: a component is *gained* from an archetype, so the first one needs
/// the empty archetype to come from.
pub fn arrives(resources: &mut Resources, entity: Entity) {
    let filed = resources
        .get::<ArchetypeRegistry>()
        .is_some_and(|archetypes| archetypes.entity_archetype(entity).is_some());
    if !filed {
        move_entity(resources, entity, ArchetypeId::EMPTY);
    }
}

/// Moves `entity` into the archetype it reaches by gaining `type_id`. A no-op for an entity that
/// is not filed anywhere yet.
pub fn gained(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) {
    let next = resources
        .get_mut::<ArchetypeRegistry>()
        .and_then(|archetypes| {
            let current = archetypes.entity_archetype(entity)?;
            Some(archetypes.archetype_after_add_dynamic(current, type_id))
        });
    if let Some(next) = next {
        move_entity(resources, entity, next);
    }
}

/// Moves `entity` into the archetype it reaches by losing `type_id`.
pub fn lost(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) {
    let next = resources
        .get_mut::<ArchetypeRegistry>()
        .and_then(|archetypes| {
            let current = archetypes.entity_archetype(entity)?;
            Some(archetypes.archetype_after_remove_dynamic(current, type_id))
        });
    if let Some(next) = next {
        move_entity(resources, entity, next);
    }
}

/// The same move, where both registries are already in hand.
///
/// Most sites that change an archetype are inside a closure holding both; the `Resources` form
/// above exists for the ones that are not.
pub fn move_in(
    components: &mut ComponentRegistry,
    archetypes: &mut ArchetypeRegistry,
    entity: Entity,
    archetype: ArchetypeId,
) {
    let settled = match stored_in(components, archetypes, archetype) {
        Some(ids) => {
            let settled = components.settle(entity, &ids);
            if let Some(table) = components.table_for(&ids)
                && let Some(held) = archetypes.get_mut(archetype)
            {
                held.set_table(table);
            }
            settled
        }
        // A component of this archetype has no registered storage, so no column can be built for
        // it. The entity keeps its values in the inbox and the dual read in `fetch` serves it.
        None => Settled {
            row: None,
            displaced: components.unsettle(entity),
        },
    };
    refile(components, archetypes, settled.displaced);
    archetypes.register_at(entity, archetype, settled.row);
}

/// Takes `T` off `entity` for good: the value, wherever it lives, and the archetype that named it.
///
/// 🔴 The only complete removal. Unparking the inbox alone leaves a settled value in its column,
/// and the component is still there — to every query, and to the inspector.
pub fn take<T: Component>(resources: &mut Resources, entity: Entity) -> bool {
    take_by_id(resources, entity, std::any::TypeId::of::<T>())
}

/// The same, for a component this binary knows only by `TypeId`.
pub fn take_by_id(resources: &mut Resources, entity: Entity, type_id: std::any::TypeId) -> bool {
    let had = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|components| {
            let had = components.has_reflector(&type_id) || components.contains_type(&type_id);
            components.remove_component(entity, &type_id);
            had
        });
    lost(resources, entity, type_id);
    had
}

/// Drops `entity`'s row and the values in it, where both registries are in hand.
pub fn drop_in(
    components: &mut ComponentRegistry,
    archetypes: &mut ArchetypeRegistry,
    entity: Entity,
) {
    let displaced = components.drop_row(entity);
    refile(components, archetypes, displaced);
    archetypes.unregister_entity(entity);
}

/// Drops `entity` from its table and from the archetype index.
pub fn drop_entity(resources: &mut Resources, entity: Entity) {
    let Some(mut components) = resources.remove::<ComponentRegistry>() else {
        return;
    };
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        drop_in(&mut components, archetypes, entity);
    }
    resources.insert(components);
}

/// Points a displaced entity's archetype slot at the row it was dragged into.
///
/// 🔴 The archetype carries a copy of the row so the walk needs no lookup, and this is the one
/// place that copy is written. Skip it and the displaced entity's query reads the row of whoever
/// took its place.
fn refile(
    components: &ComponentRegistry,
    archetypes: &mut ArchetypeRegistry,
    displaced: Option<Entity>,
) {
    if let Some(other) = displaced {
        let row = components.location(other).map(|(_, row)| row);
        archetypes.set_row(other, row);
    }
}

/// Whether every entity's archetype row agrees with where its values actually live.
///
/// 🔴 The invariant the port rests on, and the one nothing else can see breaking. An entity filed
/// by hand instead of through this funnel keeps the row the component registry gave it while its
/// archetype slot says something else — and then a query and `get_cpu` answer differently about the
/// same entity, with no error anywhere.
pub fn rows_agree(components: &ComponentRegistry, archetypes: &ArchetypeRegistry) -> bool {
    archetypes.iter_matching(&[]).all(|archetype| {
        archetype
            .entities()
            .iter()
            .zip(archetype.rows())
            .all(|(entity, row)| {
                *row == components.location(*entity).map(|(_, row)| row)
                    && (row.is_none()
                        || archetype.table()
                            == components.location(*entity).map(|(table, _)| table))
            })
    })
}

/// The dense ids of `archetype`'s components, or `None` if one of them has no storage.
fn stored_in(
    components: &ComponentRegistry,
    archetypes: &ArchetypeRegistry,
    archetype: ArchetypeId,
) -> Option<Vec<StorageId>> {
    archetypes
        .get(archetype)?
        .components()
        .iter()
        .map(|type_id| components.storage_id(type_id))
        .collect()
}

#[cfg(test)]
mod tests;
