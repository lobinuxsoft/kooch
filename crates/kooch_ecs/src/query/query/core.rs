use std::marker::PhantomData;

use kooch_core::resource::Resources;

use crate::archetype::Archetype;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::entity::Entity;

use crate::query::access::AccessTracker;
use crate::query::fetch::WorldQuery;
use crate::query::filter::QueryFilter;

use super::iter::QueryIter;

/// Type-safe query over entities with specific components.
///
/// Constructed via [`Query::new`] from `&Resources`.
///
/// # Example
///
/// ```ignore
/// fn movement_system(resources: &mut Resources) {
///     let query = Query::<(&Position, &mut Velocity), With<Player>>::new(resources);
///     for (pos, vel) in query.iter() {
///         // ...
///     }
/// }
/// ```
pub struct Query<'w, Q: WorldQuery, F: QueryFilter = ()> {
    fetch: Q::Fetch<'w>,
    matched_archetypes: Vec<&'w Archetype>,
    archetypes: &'w ArchetypeRegistry,
    tracker: &'w AccessTracker,
    _released: bool,
    _marker: PhantomData<F>,
}

impl<'w, Q: WorldQuery, F: QueryFilter> Query<'w, Q, F> {
    /// Creates a new query from the given resources.
    ///
    /// `ComponentRegistry`, `ArchetypeRegistry` and [`AccessTracker`] must
    /// all be present. [`EcsPlugin`](crate::EcsPlugin) inserts the three,
    /// so any app built the normal way already has them; a bare
    /// `Resources` assembled by hand in a test does not.
    ///
    /// This used to claim the tracker was "created automatically if not
    /// present". It never was — the line below is an `expect`.
    ///
    /// # Panics
    ///
    /// - If `ComponentRegistry` or `ArchetypeRegistry` is missing.
    /// - If a required component type is not registered.
    /// - If there is a conflicting borrow (e.g. two mutable queries on the same type).
    pub fn new(resources: &'w Resources) -> Self {
        let registry = resources
            .get::<ComponentRegistry>()
            .expect("ComponentRegistry not found in Resources");
        let archetypes = resources
            .get::<ArchetypeRegistry>()
            .expect("ArchetypeRegistry not found in Resources");
        let tracker = resources
            .get::<AccessTracker>()
            .expect("AccessTracker not found in Resources");

        // Collect required TypeIds from both the query and filter.
        let mut required = Q::required_type_ids();
        required.extend(F::required_ids());

        // Find matching archetypes.
        let matched_archetypes: Vec<&Archetype> = archetypes
            .iter_matching(&required)
            .filter(|arch| F::matches_archetype(arch))
            .filter(|arch| {
                // Exclude archetypes that contain excluded types.
                let excluded = F::excluded_ids();
                excluded.iter().all(|e| !arch.components().contains(e))
            })
            .collect();

        // SAFETY: We've verified the component types exist and the access
        // tracker will panic on conflicting borrows.
        let fetch = unsafe { Q::init_fetch(registry, tracker) };

        Self {
            fetch,
            matched_archetypes,
            archetypes,
            tracker,
            _released: false,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over all matching `(Entity, Item)` pairs.
    pub fn iter(&self) -> QueryIter<'w, '_, Q, F> {
        QueryIter {
            fetch: &self.fetch,
            archetypes: &self.matched_archetypes,
            archetype_idx: 0,
            entity_idx: 0,
            _marker: PhantomData,
        }
    }

    /// Fetches the component data for a single entity.
    ///
    /// Returns `None` if the entity doesn't have the required components
    /// or if its archetype doesn't match the query filter.
    pub fn get(&self, entity: Entity) -> Option<Q::Item<'w>> {
        // Check the entity's archetype passes the filter.
        let arch_id = self.archetypes.entity_archetype(entity)?;
        let arch = self.archetypes.get(arch_id)?;
        if !F::matches_archetype(arch) {
            return None;
        }

        // SAFETY: Borrows are tracked; the entity is validated per-storage.
        unsafe { Q::fetch(&self.fetch, entity) }
    }

    /// Returns `true` if the query matches no entities.
    pub fn is_empty(&self) -> bool {
        self.matched_archetypes.iter().all(|a| a.is_empty())
    }

    /// Applies a function to each matching entity's data.
    pub fn for_each(&self, mut func: impl FnMut(Q::Item<'w>)) {
        for archetype in &self.matched_archetypes {
            for &entity in archetype.entities() {
                // SAFETY: Archetype guarantees required components exist.
                if let Some(item) = unsafe { Q::fetch(&self.fetch, entity) } {
                    func(item);
                }
            }
        }
    }

    /// Same as [`Self::for_each`] but the closure also receives the
    /// matched [`Entity`]. Use when downstream code needs to cross-
    /// reference the iterated row against another component lookup
    /// (e.g. an optional override component checked via
    /// [`Self::get`]).
    pub fn for_each_entity(&self, mut func: impl FnMut(Entity, Q::Item<'w>)) {
        for archetype in &self.matched_archetypes {
            for &entity in archetype.entities() {
                // SAFETY: Archetype guarantees required components exist.
                if let Some(item) = unsafe { Q::fetch(&self.fetch, entity) } {
                    func(entity, item);
                }
            }
        }
    }
}

impl<Q: WorldQuery, F: QueryFilter> Drop for Query<'_, Q, F> {
    fn drop(&mut self) {
        if !self._released {
            Q::release_fetch(self.tracker);
            self._released = true;
        }
    }
}
