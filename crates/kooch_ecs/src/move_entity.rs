//! Moving an entity between archetypes, row and filing together (#891).

use kooch_core::resource::Resources;

use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::{ComponentRegistry, StorageId};
use crate::entity::Entity;

/// Moves `entity` into `archetype`: its values first, then the archetype it is filed under.
///
/// 🔴 Both halves or neither. An archetype changed without the row moving leaves that entity
/// pointing at the table it used to be in, where it reads **another entity's components** — no
/// error, no warning. That is why every site that changes an archetype goes through here.
pub fn move_entity(resources: &mut Resources, entity: Entity, archetype: ArchetypeId) {
    let Some(mut components) = resources.remove::<ComponentRegistry>() else {
        return;
    };
    let ids = resources
        .get::<ArchetypeRegistry>()
        .and_then(|archetypes| ids_of(archetypes, archetype, &components));
    if let Some(ids) = ids {
        components.relocate(entity, &ids);
    }
    resources.insert(components);
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.register_entity(entity, archetype);
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
    if let Some(ids) = ids_of(archetypes, archetype, components) {
        components.relocate(entity, &ids);
    }
    archetypes.register_entity(entity, archetype);
}

/// Drops `entity` from its table and its row, where both registries are in hand.
pub fn drop_in(
    components: &mut ComponentRegistry,
    archetypes: &mut ArchetypeRegistry,
    entity: Entity,
) {
    components.evict(entity);
    archetypes.unregister_entity(entity);
}

/// Drops `entity` from its table and from the archetype index.
pub fn drop_entity(resources: &mut Resources, entity: Entity) {
    if let Some(components) = resources.get_mut::<ComponentRegistry>() {
        components.evict(entity);
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.unregister_entity(entity);
    }
}

/// The archetype's component types as storage ids, or `None` when one is not registered — a table
/// cannot be built for a component nothing knows how to store.
fn ids_of(
    archetypes: &ArchetypeRegistry,
    archetype: ArchetypeId,
    components: &ComponentRegistry,
) -> Option<Vec<StorageId>> {
    archetypes
        .get(archetype)?
        .components()
        .iter()
        .map(|type_id| components.storage_id(type_id))
        .collect()
}
