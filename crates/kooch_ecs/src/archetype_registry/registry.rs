use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};

use crate::archetype::{Archetype, ArchetypeId};
use crate::entity::Entity;
use crate::storage::TableRow;

use super::filed::Filed;

/// Where an entity is filed: which archetype, and which slot of it.
///
/// The slot is what makes leaving O(1). Without it, every archetype change scanned the entity list
/// to find the entity it was about to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Where {
    /// The archetype the entity belongs to.
    pub archetype: ArchetypeId,
    /// Its index into that archetype's parallel entity and row lists.
    pub at: u32,
}

/// Central registry of all archetypes and entity-archetype mappings.
pub struct ArchetypeRegistry {
    /// All known archetypes.
    archetypes: HashMap<ArchetypeId, Archetype>,
    /// Entity → the archetype and slot holding it.
    located: Filed,
    /// Cache: `(from_archetype, +component_type)` → `to_archetype`.
    add_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
    /// Cache: `(from_archetype, -component_type)` → `to_archetype`.
    remove_transitions: HashMap<(ArchetypeId, TypeId), ArchetypeId>,
    /// Bumped by every structural change — an entity registered, moved between archetypes or
    /// evicted. Read by anything caching a view of the world's shape (#1561).
    revision: u64,
}

impl ArchetypeRegistry {
    /// Creates a new registry with the empty archetype pre-registered.
    pub fn new() -> Self {
        let mut archetypes = HashMap::new();
        archetypes.insert(ArchetypeId::EMPTY, Archetype::new(BTreeSet::new()));

        Self {
            archetypes,
            located: Filed::default(),
            add_transitions: HashMap::new(),
            remove_transitions: HashMap::new(),
            revision: 0,
        }
    }

    /// How many structural changes this registry has seen.
    #[inline]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the archetype for a component set, creating it if needed.
    pub fn get_or_create(&mut self, components: BTreeSet<TypeId>) -> ArchetypeId {
        let id = ArchetypeId::from_components(&components);

        // Only the `create` half is a change. This is a lookup on every call where the archetype
        // already exists, and bumping there invalidated the editor's snapshot every frame (#1561).
        if let std::collections::hash_map::Entry::Vacant(slot) = self.archetypes.entry(id) {
            slot.insert(Archetype::new(components));
            self.revision += 1;
        }

        id
    }

    /// Returns an immutable reference to an archetype.
    pub fn get(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(&id)
    }

    /// An archetype, mutably.
    pub fn get_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_mut(&id)
    }

    /// Returns the archetype an entity currently belongs to.
    pub fn entity_archetype(&self, entity: Entity) -> Option<ArchetypeId> {
        self.located.get(entity).map(|at| at.archetype)
    }

    /// Where `entity` is filed, archetype and slot.
    #[inline]
    pub fn located(&self, entity: Entity) -> Option<Where> {
        self.located.get(entity)
    }

    /// The table row holding `entity`'s values, if it has one.
    #[inline]
    pub fn row_of(&self, entity: Entity) -> Option<TableRow> {
        let at = self.located(entity)?;
        self.archetypes.get(&at.archetype)?.row_at(at.at)
    }

    /// Moves an entity into `archetype_id`, recording which table row holds its values.
    ///
    /// 🔴 The only safe caller for a world that holds component values is
    /// [`move_entity::move_in`](crate::move_entity::move_in). The row recorded here and the one
    /// [`ComponentRegistry`](crate::ComponentRegistry) holds are the same fact written twice — one
    /// for the walk, which must not hash, and one for a reader that has no archetype in hand. The
    /// funnel is what keeps them equal; filing an entity here on its own does not, and the
    /// disagreement is silent: the query reads one answer and `get_cpu` the other. Check it with
    /// [`move_entity::rows_agree`](crate::move_entity::rows_agree).
    ///
    /// 🔴 Leaving a slot drags the archetype's last entity into it. That entity asked for nothing
    /// and its slot changed anyway, so it is re-filed here — miss it and its next move removes the
    /// wrong entity from the list.
    pub fn register_at(
        &mut self,
        entity: Entity,
        archetype_id: ArchetypeId,
        row: Option<TableRow>,
    ) {
        if let Some(old) = self.located.get(entity) {
            if old.archetype == archetype_id {
                // Already there: only the row can have changed, and that is not a structural
                // change — nothing watching the world's shape has to look again.
                if let Some(archetype) = self.archetypes.get_mut(&archetype_id) {
                    archetype.set_row(old.at, row);
                }
                return;
            }
            self.vacate(old);
        }

        let at = match self.archetypes.get_mut(&archetype_id) {
            Some(archetype) => archetype.add_entity(entity, row),
            None => 0,
        };
        self.located.set(entity, archetype_id, at);
        self.revision += 1;
    }

    /// Points `entity`'s slot at a different table row. Not a structural change.
    pub fn set_row(&mut self, entity: Entity, row: Option<TableRow>) {
        let Some(at) = self.located(entity) else {
            return;
        };
        if let Some(archetype) = self.archetypes.get_mut(&at.archetype) {
            archetype.set_row(at.at, row);
        }
    }

    /// Removes an entity from its current archetype entirely.
    ///
    /// Returns the archetype it was removed from, or `None` if untracked.
    pub fn unregister_entity(&mut self, entity: Entity) -> Option<ArchetypeId> {
        let at = self.located.take(entity)?;
        self.revision += 1;
        self.vacate(at);
        Some(at.archetype)
    }

    /// Frees a slot, re-filing whichever entity got dragged into it.
    fn vacate(&mut self, at: Where) {
        let dragged = self
            .archetypes
            .get_mut(&at.archetype)
            .and_then(|archetype| archetype.remove_at(at.at));
        if let Some(other) = dragged {
            self.located.set(other, at.archetype, at.at);
        }
    }

    /// Computes the archetype that results from adding component `T` to
    /// `current`, using the transition cache.
    pub fn archetype_after_add<T: 'static>(&mut self, current: ArchetypeId) -> ArchetypeId {
        self.archetype_after_add_dynamic(current, TypeId::of::<T>())
    }

    /// Computes the archetype that results from removing component `T` from
    /// `current`, using the transition cache.
    pub fn archetype_after_remove<T: 'static>(&mut self, current: ArchetypeId) -> ArchetypeId {
        self.archetype_after_remove_dynamic(current, TypeId::of::<T>())
    }

    /// Like [`archetype_after_add`](Self::archetype_after_add) but takes a
    /// `TypeId` directly.
    pub fn archetype_after_add_dynamic(
        &mut self,
        current: ArchetypeId,
        type_id: TypeId,
    ) -> ArchetypeId {
        let key = (current, type_id);

        if let Some(&cached) = self.add_transitions.get(&key) {
            return cached;
        }

        let mut new_components = self.archetypes[&current].components().clone();
        new_components.insert(type_id);

        let new_id = self.get_or_create(new_components);
        self.add_transitions.insert(key, new_id);
        new_id
    }

    /// Like [`archetype_after_remove`](Self::archetype_after_remove) but takes
    /// a `TypeId` directly.
    pub fn archetype_after_remove_dynamic(
        &mut self,
        current: ArchetypeId,
        type_id: TypeId,
    ) -> ArchetypeId {
        let key = (current, type_id);

        if let Some(&cached) = self.remove_transitions.get(&key) {
            return cached;
        }

        let mut new_components = self.archetypes[&current].components().clone();
        new_components.remove(&type_id);

        let new_id = self.get_or_create(new_components);
        self.remove_transitions.insert(key, new_id);
        new_id
    }

    /// Iterates over archetypes that contain **all** of the `required`
    /// component types.
    pub fn iter_matching(&self, required: &[TypeId]) -> impl Iterator<Item = &Archetype> {
        self.archetypes
            .values()
            .filter(move |arch| required.iter().all(|r| arch.components().contains(r)))
    }

    /// Reorders every archetype's entities to follow `order`.
    pub fn reorder_entities(&mut self, order: &[Entity]) {
        self.revision += 1;
        let rank: HashMap<Entity, usize> = order.iter().enumerate().map(|(i, e)| (*e, i)).collect();
        for archetype in self.archetypes.values_mut() {
            archetype.reorder_entities(&rank);
        }
        // The slots moved with the entities, so every one of them is stale.
        self.refile();
    }

    /// Rebuilds the entity → slot index from the archetypes themselves.
    fn refile(&mut self) {
        self.located.clear();
        for (&archetype, held) in &self.archetypes {
            for (at, &entity) in held.entities().iter().enumerate() {
                self.located.set(entity, archetype, at as u32);
            }
        }
    }

    /// Returns the total number of registered archetypes.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// Removes empty archetypes (except `EMPTY`) and invalidates transition cache entries that
    /// reference them.
    pub fn gc_empty_archetypes(&mut self) -> usize {
        let to_remove: Vec<ArchetypeId> = self
            .archetypes
            .iter()
            .filter(|(id, arch)| **id != ArchetypeId::EMPTY && arch.len() == 0)
            .map(|(&id, _)| id)
            .collect();

        let count = to_remove.len();
        if count > 0 {
            self.revision += 1;
        }
        for id in &to_remove {
            self.archetypes.remove(id);
        }

        // Purge transition cache entries that reference removed archetypes.
        self.add_transitions
            .retain(|&(src, _), dst| !to_remove.contains(&src) && !to_remove.contains(dst));
        self.remove_transitions
            .retain(|&(src, _), dst| !to_remove.contains(&src) && !to_remove.contains(dst));

        count
    }
}

impl Default for ArchetypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
