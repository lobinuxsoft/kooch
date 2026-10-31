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
    archetypes.register_at(entity, ArchetypeId::EMPTY, None);
    entity
}

/// The real gesture: write the value, then move the archetype through the funnel — which is what
/// promotes the value into its column.
fn add_cpu_component<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    let components = resources.get_mut::<ComponentRegistry>().unwrap();
    components.register_cpu::<T>();
    components.get_cpu_mut::<T>().unwrap().insert(entity, value);
    crate::move_entity::gained(resources, entity, std::any::TypeId::of::<T>());
}

fn add_component<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    add_cpu_component(resources, entity, value);
}

/// The same component, left in the inbox: the archetype knows about it but no column was built,
/// which is the state of everything a test registers by hand.
fn park_component<T: Component>(resources: &mut Resources, entity: Entity, value: T) {
    let components = resources.get_mut::<ComponentRegistry>().unwrap();
    components.register_cpu::<T>();
    components.get_cpu_mut::<T>().unwrap().insert(entity, value);

    let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
    let current = archetypes.entity_archetype(entity).unwrap();
    let new_arch = archetypes.archetype_after_add::<T>(current);
    archetypes.register_at(entity, new_arch, None);
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

/// Two `&mut` of **different** components in one query is fine — the tracker counts borrows per
/// component type, not per query.
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

/// Eight is the arity ceiling: `impl_world_query_tuple!` is instantiated up to `H` (`fetch.rs`).
/// All eight may be `&mut` — the tracker counts per component type, so distinct types never
/// conflict no matter how many.
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

// -- Reading from a column (#891) ------------------------------------------

mod columns {
    use super::*;

    /// 🔴 The proof the whole port rests on: after the normal route, the value is in the column and
    /// the inbox is empty, so a query that still read the map would see nothing.
    #[test]
    fn the_value_left_the_inbox() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        add_cpu_component(&mut r, entity, Health(77));

        let components = r.get::<ComponentRegistry>().unwrap();
        let (table, row) = components.location(entity).expect("it settled");
        let id = components
            .storage_id(&std::any::TypeId::of::<Health>())
            .unwrap();
        let column = components
            .tables()
            .get(table)
            .unwrap()
            .column(id)
            .expect("its column exists");

        assert_eq!(unsafe { column.get::<Health>(row.index()) }.unwrap().0, 77);
    }

    #[test]
    fn for_each_reads_the_column() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        add_cpu_component(&mut r, entity, Health(77));

        let mut seen = Vec::new();
        Query::<&Health>::new(&r).for_each(|h| seen.push(h.0));

        assert_eq!(seen, vec![77], "read from the column, not the map");
    }

    /// 🔴 The iterator and `for_each` must read the same place. One following the column and the
    /// other not would make an entity visible to half the engine and absent from the other half,
    /// silently.
    #[test]
    fn iter_reads_the_column_too() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        add_cpu_component(&mut r, entity, Health(77));

        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![77]);
    }

    #[test]
    fn a_mutable_query_reaches_the_column() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        add_cpu_component(&mut r, entity, Health(77));

        Query::<&mut Health>::new(&r).for_each(|h| h.0 += 1);
        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![78], "the write landed in the column");
    }

    /// The inbox is still the answer for an entity filed by hand, which every test that builds a
    /// world without the funnel does.
    #[test]
    fn the_inbox_still_answers() {
        let mut r = setup();
        let entity = spawn_entity(&mut r, 1);
        park_component(&mut r, entity, Health(5));

        let seen: Vec<u32> = Query::<&Health>::new(&r).iter().map(|h| h.0).collect();

        assert_eq!(seen, vec![5]);
    }

    /// 🔴 The acceptance of #891, and the one that cannot be checked by reading the code: after a
    /// row moved out of the middle of a table, every entity still reads **its own** values through
    /// a two-component query. Get the displaced entity wrong and this reads a neighbour's.
    #[test]
    fn a_displaced_row_still_reads_itself() {
        let mut r = setup();
        let held: Vec<Entity> = (1..6).map(|i| spawn_entity(&mut r, i)).collect();
        for (i, &entity) in held.iter().enumerate() {
            add_cpu_component(&mut r, entity, Health(i as u32));
            add_cpu_component(&mut r, entity, Name(format!("n{i}")));
        }
        // Out of the middle, so the last row is dragged into the hole it leaves.
        add_cpu_component(&mut r, held[1], Marker);

        let mut seen: Vec<(u32, String)> = Vec::new();
        Query::<(&Health, &Name)>::new(&r).for_each(|(h, n)| seen.push((h.0, n.0.clone())));
        seen.sort();

        assert_eq!(
            seen,
            (0..5)
                .map(|i| (i as u32, format!("n{i}")))
                .collect::<Vec<_>>(),
            "a health read a neighbour's name"
        );
    }

    /// A column and an inbox at once: two entities of the same archetype, one settled and one not.
    /// Both have to come out of the same walk.
    #[test]
    fn both_homes_in_one_walk() {
        let mut r = setup();
        let settled = spawn_entity(&mut r, 1);
        let parked = spawn_entity(&mut r, 2);
        add_cpu_component(&mut r, settled, Health(10));
        park_component(&mut r, parked, Health(20));

        let mut seen: Vec<u32> = Vec::new();
        Query::<&Health>::new(&r).for_each(|h| seen.push(h.0));
        seen.sort_unstable();

        assert_eq!(seen, vec![10, 20]);
    }
}

/// An optional component's borrow ends with its query, or the next frame's query panics (#1722).
#[test]
fn an_optional_borrow_is_released() {
    let mut resources = setup();
    let entity = spawn_entity(&mut resources, 0);
    add_cpu_component(&mut resources, entity, Name("n".into()));
    add_cpu_component(&mut resources, entity, Health(1));
    for _ in 0..2 {
        Query::<(&Name, Option<&mut Health>)>::new(&resources).for_each(|_| {});
    }
    Query::<&mut Health>::new(&resources).for_each(|health| health.0 += 1);
}
