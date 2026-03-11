//! Main `Query` type and iterator.
//!
//! [`Query`] is constructed from [`Resources`] and provides ergonomic,
//! type-safe iteration over entities matching a component + filter pattern.

use std::marker::PhantomData;

use ome_core::resource::Resources;

use crate::archetype::Archetype;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::entity::Entity;

use super::access::AccessTracker;
use super::fetch::WorldQuery;
use super::filter::QueryFilter;

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
    tracker: &'w AccessTracker,
    _released: bool,
    _marker: PhantomData<F>,
}

impl<'w, Q: WorldQuery, F: QueryFilter> Query<'w, Q, F> {
    /// Creates a new query from the given resources.
    ///
    /// The `ComponentRegistry` and `ArchetypeRegistry` must be present
    /// in resources. An [`AccessTracker`] is created automatically if not
    /// present.
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
                excluded
                    .iter()
                    .all(|e| !arch.components().contains(e))
            })
            .collect();

        // SAFETY: We've verified the component types exist and the access
        // tracker will panic on conflicting borrows.
        let fetch = unsafe { Q::init_fetch(registry, tracker) };

        Self {
            fetch,
            matched_archetypes,
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
    /// Returns `None` if the entity doesn't have the required components.
    pub fn get(&self, entity: Entity) -> Option<Q::Item<'w>> {
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
}

impl<Q: WorldQuery, F: QueryFilter> Drop for Query<'_, Q, F> {
    fn drop(&mut self) {
        if !self._released {
            Q::release_fetch(self.tracker);
            self._released = true;
        }
    }
}

/// Iterator over entities matching a [`Query`].
pub struct QueryIter<'w, 'q, Q: WorldQuery, F: QueryFilter> {
    fetch: &'q Q::Fetch<'w>,
    archetypes: &'q [&'w Archetype],
    archetype_idx: usize,
    entity_idx: usize,
    _marker: PhantomData<F>,
}

impl<'w, 'q, Q: WorldQuery, F: QueryFilter> Iterator for QueryIter<'w, 'q, Q, F> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.archetype_idx >= self.archetypes.len() {
                return None;
            }

            let archetype = self.archetypes[self.archetype_idx];
            let entities = archetype.entities();

            if self.entity_idx >= entities.len() {
                self.archetype_idx += 1;
                self.entity_idx = 0;
                continue;
            }

            let entity = entities[self.entity_idx];
            self.entity_idx += 1;

            // SAFETY: Archetype matching guarantees required components exist.
            // The fetch was properly initialised with valid borrows.
            if let Some(item) = unsafe { Q::fetch(self.fetch, entity) } {
                return Some(item);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining: usize = self.archetypes[self.archetype_idx..]
            .iter()
            .map(|a| a.entities().len())
            .sum::<usize>()
            .saturating_sub(self.entity_idx);
        (0, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use ome_core::resource::Resources;

    use crate::archetype::ArchetypeId;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::component::{Component, ComponentRegistry, GpuComponent};
    use crate::entity::Entity;
    use crate::query::access::AccessTracker;
    use crate::query::filter::{With, Without};
    use crate::query::query::Query;

    // -- Test components --

    struct Health(u32);
    impl Component for Health {}

    struct Name(String);
    impl Component for Name {}

    struct Marker;
    impl Component for Marker {}

    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct Position {
        x: f32,
        y: f32,
    }
    impl GpuComponent for Position {}

    // -- Helpers --

    fn setup() -> Resources {
        let mut resources = Resources::new();
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());
        resources
    }

    fn spawn_entity(resources: &mut Resources, index: u32) -> Entity {
        let entity = Entity::new(index, 0);
        let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
        archetypes.register_entity(entity, ArchetypeId::EMPTY);
        entity
    }

    fn add_cpu_component<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
        let components = resources.get_mut::<ComponentRegistry>().unwrap();
        components.register_cpu::<T>();
        components.get_cpu_mut::<T>().unwrap().insert(entity, value);

        // Update archetype
        let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
        let current = archetypes.entity_archetype(entity).unwrap();
        let new_arch = archetypes.archetype_after_add::<T>(current);
        archetypes.register_entity(entity, new_arch);
    }

    fn add_gpu_component<T: GpuComponent>(
        resources: &mut Resources,
        entity: Entity,
        value: T,
        label: &str,
    ) {
        let components = resources.get_mut::<ComponentRegistry>().unwrap();
        components.register_gpu::<T>(label);
        components.get_gpu_mut::<T>().unwrap().insert(entity, value);

        let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
        let current = archetypes.entity_archetype(entity).unwrap();
        let new_arch = archetypes.archetype_after_add::<T>(current);
        archetypes.register_entity(entity, new_arch);
    }

    // -- Tests --

    #[test]
    fn query_single_cpu_component() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        let e1 = spawn_entity(&mut resources, 1);

        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e1, Health(50));

        let query = Query::<&Health>::new(&resources);
        let results: Vec<&Health> = query.iter().collect();
        assert_eq!(results.len(), 2);

        let sum: u32 = results.iter().map(|h| h.0).sum();
        assert_eq!(sum, 150);
    }

    #[test]
    fn query_mutable_cpu_component() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        {
            let query = Query::<&mut Health>::new(&resources);
            query.for_each(|h| {
                h.0 += 50;
            });
        }

        let query = Query::<&Health>::new(&resources);
        let health = query.iter().next().unwrap();
        assert_eq!(health.0, 150);
    }

    #[test]
    fn query_gpu_component_read_only() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_gpu_component(
            &mut resources,
            e0,
            Position { x: 1.0, y: 2.0 },
            "position",
        );

        let query = Query::<&Position>::new(&resources);
        let pos = query.iter().next().unwrap();
        assert_eq!(pos.x, 1.0);
        assert_eq!(pos.y, 2.0);
    }

    #[test]
    #[should_panic(expected = "storage is read-only (GPU component)")]
    fn query_gpu_component_mutable_panics() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_gpu_component(
            &mut resources,
            e0,
            Position { x: 1.0, y: 2.0 },
            "position",
        );

        // Should panic: GPU components are read-only from queries.
        let _query = Query::<&mut Position>::new(&resources);
    }

    #[test]
    fn query_with_filter() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        let e1 = spawn_entity(&mut resources, 1);

        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e0, Marker);
        add_cpu_component(&mut resources, e1, Health(50));

        let query = Query::<&Health, With<Marker>>::new(&resources);
        let results: Vec<&Health> = query.iter().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 100);
    }

    #[test]
    fn query_without_filter() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        let e1 = spawn_entity(&mut resources, 1);

        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e0, Marker);
        add_cpu_component(&mut resources, e1, Health(50));

        let query = Query::<&Health, Without<Marker>>::new(&resources);
        let results: Vec<&Health> = query.iter().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 50);
    }

    #[test]
    fn query_entity_component() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        let query = Query::<(Entity, &Health)>::new(&resources);
        let (entity, health) = query.iter().next().unwrap();
        assert_eq!(entity, e0);
        assert_eq!(health.0, 100);
    }

    #[test]
    fn query_multiple_components() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e0, Name("Alice".into()));

        let query = Query::<(&Health, &Name)>::new(&resources);
        let (health, name) = query.iter().next().unwrap();
        assert_eq!(health.0, 100);
        assert_eq!(name.0, "Alice");
    }

    #[test]
    fn query_mixed_cpu_gpu() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));
        add_gpu_component(
            &mut resources,
            e0,
            Position { x: 3.0, y: 4.0 },
            "position",
        );

        let query = Query::<(&Health, &Position)>::new(&resources);
        let (health, pos) = query.iter().next().unwrap();
        assert_eq!(health.0, 100);
        assert_eq!(pos.x, 3.0);
    }

    #[test]
    fn query_get_single_entity() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        let e1 = spawn_entity(&mut resources, 1);
        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e1, Health(50));

        let query = Query::<&Health>::new(&resources);
        assert_eq!(query.get(e0).unwrap().0, 100);
        assert_eq!(query.get(e1).unwrap().0, 50);
    }

    #[test]
    fn query_empty_result() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        // Query for Name, which e0 doesn't have
        let query = Query::<&Name>::new(&resources);
        assert!(query.is_empty());
        assert_eq!(query.iter().count(), 0);
    }

    #[test]
    fn query_multiple_archetypes() {
        let mut resources = setup();

        // e0: Health only
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        // e1: Health + Name
        let e1 = spawn_entity(&mut resources, 1);
        add_cpu_component(&mut resources, e1, Health(50));
        add_cpu_component(&mut resources, e1, Name("Bob".into()));

        // e2: Health + Marker
        let e2 = spawn_entity(&mut resources, 2);
        add_cpu_component(&mut resources, e2, Health(25));
        add_cpu_component(&mut resources, e2, Marker);

        // Query<&Health> should match all 3 entities across 3 archetypes.
        let query = Query::<&Health>::new(&resources);
        let total: u32 = query.iter().map(|h| h.0).sum();
        assert_eq!(total, 175);
    }

    #[test]
    fn sequential_queries_release_borrows() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        // First mutable query — borrows Health exclusively.
        {
            let query = Query::<&mut Health>::new(&resources);
            query.for_each(|h| h.0 = 200);
        } // Borrow released on drop.

        // Second mutable query — should NOT panic.
        {
            let query = Query::<&mut Health>::new(&resources);
            let h = query.get(e0).unwrap();
            assert_eq!(h.0, 200);
        }
    }

    #[test]
    fn concurrent_read_queries() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));
        add_cpu_component(&mut resources, e0, Name("Test".into()));

        // Two read queries on different types should coexist.
        let q1 = Query::<&Health>::new(&resources);
        let q2 = Query::<&Name>::new(&resources);

        assert_eq!(q1.get(e0).unwrap().0, 100);
        assert_eq!(q2.get(e0).unwrap().0, "Test");
    }

    #[test]
    fn concurrent_read_same_type() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        // Two read queries on the SAME type should coexist.
        let q1 = Query::<&Health>::new(&resources);
        let q2 = Query::<&Health>::new(&resources);

        assert_eq!(q1.get(e0).unwrap().0, 100);
        assert_eq!(q2.get(e0).unwrap().0, 100);
    }

    #[test]
    #[should_panic(expected = "cannot borrow component as mutable: already borrowed")]
    fn conflicting_borrow_panics() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        add_cpu_component(&mut resources, e0, Health(100));

        let _q1 = Query::<&Health>::new(&resources);
        // This should panic: Health is already borrowed immutably.
        let _q2 = Query::<&mut Health>::new(&resources);
    }

    #[test]
    fn for_each_works() {
        let mut resources = setup();
        let e0 = spawn_entity(&mut resources, 0);
        let e1 = spawn_entity(&mut resources, 1);
        add_cpu_component(&mut resources, e0, Health(10));
        add_cpu_component(&mut resources, e1, Health(20));

        let query = Query::<&Health>::new(&resources);
        let mut sum = 0u32;
        query.for_each(|h| sum += h.0);
        assert_eq!(sum, 30);
    }
}
