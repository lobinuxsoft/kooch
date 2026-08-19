use kooch_core::resource::Resources;

use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::{Component, ComponentRegistry};
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
impl Component for Position {}

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

fn add_component<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    let components = resources.get_mut::<ComponentRegistry>().unwrap();
    components.register_cpu::<T>();
    components.get_cpu_mut::<T>().unwrap().insert(entity, value);

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
    add_component(&mut resources, e0, Position { x: 1.0, y: 2.0 });

    let query = Query::<&Position>::new(&resources);
    let pos = query.iter().next().unwrap();
    assert_eq!(pos.x, 1.0);
    assert_eq!(pos.y, 2.0);
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
    add_component(&mut resources, e0, Position { x: 3.0, y: 4.0 });

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

/// Two `&mut` of **different** components in one query is fine — the
/// tracker counts borrows per component type, not per query.
///
/// Worth pinning because the natural assumption is the opposite, and
/// acting on it produces two queries and a manual join where one query
/// would do.
#[test]
fn one_query_can_hold_two_mutable_components() {
    let mut resources = setup();
    let e0 = spawn_entity(&mut resources, 0);
    add_cpu_component(&mut resources, e0, Health(100));
    add_cpu_component(&mut resources, e0, Name("Alice".into()));

    {
        let query = Query::<(&mut Health, &mut Name)>::new(&resources);
        query.for_each(|(health, name)| {
            health.0 += 1;
            name.0 = "Bob".into();
        });
    }

    let query = Query::<(&Health, &Name)>::new(&resources);
    let (health, name) = query.iter().next().unwrap();
    assert_eq!(health.0, 101);
    assert_eq!(name.0, "Bob");
}

/// And the thing that actually conflicts: the **same** component held
/// mutably twice, whether by two queries or one.
#[test]
#[should_panic(expected = "already borrowed")]
fn the_same_component_cannot_be_held_mutably_twice() {
    let mut resources = setup();
    let e0 = spawn_entity(&mut resources, 0);
    add_cpu_component(&mut resources, e0, Health(100));

    let _first = Query::<&mut Health>::new(&resources);
    let _second = Query::<&mut Health>::new(&resources);
}

/// Eight is the arity ceiling: `impl_world_query_tuple!` is instantiated
/// up to `H` (`fetch.rs`). All eight may be `&mut` — the tracker counts
/// per component type, so distinct types never conflict no matter how
/// many. `Entity` and filters cost nothing here; `Entity` occupies a slot,
/// `With`/`Without` live in the second parameter.
#[test]
fn eight_mutable_components_fit_in_one_query() {
    #[derive(Debug)]
    struct C1(u32);
    #[derive(Debug)]
    struct C2(u32);
    #[derive(Debug)]
    struct C3(u32);
    #[derive(Debug)]
    struct C4(u32);
    #[derive(Debug)]
    struct C5(u32);
    #[derive(Debug)]
    struct C6(u32);
    #[derive(Debug)]
    struct C7(u32);
    #[derive(Debug)]
    struct C8(u32);
    impl Component for C1 {}
    impl Component for C2 {}
    impl Component for C3 {}
    impl Component for C4 {}
    impl Component for C5 {}
    impl Component for C6 {}
    impl Component for C7 {}
    impl Component for C8 {}

    let mut resources = setup();
    let e0 = spawn_entity(&mut resources, 0);
    add_cpu_component(&mut resources, e0, C1(1));
    add_cpu_component(&mut resources, e0, C2(2));
    add_cpu_component(&mut resources, e0, C3(3));
    add_cpu_component(&mut resources, e0, C4(4));
    add_cpu_component(&mut resources, e0, C5(5));
    add_cpu_component(&mut resources, e0, C6(6));
    add_cpu_component(&mut resources, e0, C7(7));
    add_cpu_component(&mut resources, e0, C8(8));

    {
        let query = Query::<(
            &mut C1,
            &mut C2,
            &mut C3,
            &mut C4,
            &mut C5,
            &mut C6,
            &mut C7,
            &mut C8,
        )>::new(&resources);
        query.for_each(|(a, b, c, d, e, f, g, h)| {
            a.0 += 10;
            b.0 += 10;
            c.0 += 10;
            d.0 += 10;
            e.0 += 10;
            f.0 += 10;
            g.0 += 10;
            h.0 += 10;
        });
    }

    let query = Query::<(&C1, &C8)>::new(&resources);
    let (first, last) = query.iter().next().unwrap();
    assert_eq!((first.0, last.0), (11, 18), "not every slot was written");
}

// -- Reading from a column (#891, stage 5d-a) -------------------------------

mod columns {
    use std::any::TypeId;
    use std::collections::BTreeSet;

    use super::*;

    /// Puts `entity` in an archetype holding `Health`, with the value in
    /// the **column and nowhere else**: the per-type map is cleared after
    /// the row is filled.
    ///
    /// 🔴 That is what makes this test worth writing. If the value were in
    /// both, a pass would prove nothing — the fallback would answer and
    /// look identical. With the map empty, only the column can.
    fn only_in_a_column(resources: &mut Resources, entity: Entity, value: u32) {
        // Taken out and put back: `place` wants the component registry by
        // shared reference and the archetype registry by exclusive one, and
        // `Resources` cannot hand out both at once. The real insert path
        // does not have this problem — it already receives the two as
        // separate parameters.
        let mut components = resources.remove::<ComponentRegistry>().unwrap();
        components.register_cpu::<Health>();
        let health = components.storage_id(&TypeId::of::<Health>()).unwrap();

        let set: BTreeSet<TypeId> = [TypeId::of::<Health>()].into_iter().collect();
        {
            let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
            let archetype = archetypes.get_or_create(set);
            archetypes.place(entity, archetype, &components).unwrap();
            let table = archetypes.table_of(archetype, &components).unwrap();
            unsafe {
                archetypes
                    .tables_mut()
                    .get_mut(table)
                    .unwrap()
                    .column_mut(health)
                    .unwrap()
                    .push(Health(value))
            };
        }

        // The map holds nothing for this entity, so only the column can
        // answer. That is the whole point of the fixture.
        resources.insert(components);
    }

    #[test]
    fn for_each_reads_the_column() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        only_in_a_column(&mut r, entity, 77);

        let mut seen = Vec::new();
        Query::<&Health>::new(&r).for_each(|h| seen.push(h.0));

        assert_eq!(seen, vec![77], "read from the column, not the map");
    }

    /// 🔴 The iterator and `for_each` must read the same place. One
    /// following the column and the other not would make an entity visible
    /// to half the engine and absent from the other half, silently.
    #[test]
    fn iter_reads_the_column_too() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        only_in_a_column(&mut r, entity, 77);

        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![77]);
    }

    #[test]
    fn a_mutable_query_reaches_the_column() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        only_in_a_column(&mut r, entity, 77);

        Query::<&mut Health>::new(&r).for_each(|h| h.0 += 1);
        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![78], "the write landed in the column");
    }

    /// The map is still the answer for everything that has not moved, which
    /// during the migration is everything.
    #[test]
    fn the_map_still_answers_when_there_is_no_column() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        add_cpu_component(&mut r, entity, Health(5));

        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![5]);
    }
}
