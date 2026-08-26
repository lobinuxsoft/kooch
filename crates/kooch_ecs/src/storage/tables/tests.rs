use super::Tables;
use crate::component::traits::Component;
use crate::component::{ComponentRegistry, StorageId};

struct Health(u32);
impl Component for Health {}

struct Speed(f32);
impl Component for Speed {}

struct Armour(u8);
impl Component for Armour {}

/// A registry with the three test components, and their ids.
fn registry() -> (ComponentRegistry, StorageId, StorageId, StorageId) {
    let mut registry = ComponentRegistry::new();
    let health = registry.register_cpu::<Health>();
    let speed = registry.register_cpu::<Speed>();
    let armour = registry.register_cpu::<Armour>();
    (registry, health, speed, armour)
}

#[test]
fn a_set_gets_a_table_with_its_columns() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();

    let id = tables.get_or_insert(&registry, &[health, speed]);
    let table = tables.get(id).unwrap();

    assert!(table.column(health).is_some());
    assert!(table.column(speed).is_some());
    assert!(table.is_empty());
    assert_eq!(tables.len(), 1);
}

/// The lookup is by component SET, so the second ask reuses the first
/// table rather than building a parallel one nobody would notice.
#[test]
fn the_same_set_reuses_its_table() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();

    let first = tables.get_or_insert(&registry, &[health, speed]);
    let again = tables.get_or_insert(&registry, &[health, speed]);

    assert_eq!(first, again);
    assert_eq!(tables.len(), 1);
}

/// 🔴 A set is a set: the caller's ordering is not part of the identity.
/// Two archetypes listing the same components in different orders must
/// not end up with two tables holding the same thing.
#[test]
fn order_is_not_part_of_the_identity() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();

    let forwards = tables.get_or_insert(&registry, &[health, speed]);
    let backwards = tables.get_or_insert(&registry, &[speed, health]);

    assert_eq!(forwards, backwards);
    assert_eq!(tables.len(), 1);
}

/// A component named twice is still one column, and `Table::new` would
/// panic on the duplicate if it reached that far.
#[test]
fn a_repeated_component_is_one_column() {
    let (registry, health, _, _) = registry();
    let mut tables = Tables::new();

    let id = tables.get_or_insert(&registry, &[health, health]);

    assert_eq!(tables.get(id).unwrap().component_ids(), &[health]);
}

#[test]
fn different_sets_get_different_tables() {
    let (registry, health, speed, armour) = registry();
    let mut tables = Tables::new();

    let one = tables.get_or_insert(&registry, &[health, speed]);
    let two = tables.get_or_insert(&registry, &[health, armour]);
    let three = tables.get_or_insert(&registry, &[health]);

    assert_ne!(one, two);
    assert_ne!(two, three);
    assert_eq!(tables.len(), 3);
}

/// An entity with no components is a legitimate thing to spawn, and the
/// empty set is a legitimate table.
#[test]
fn the_empty_set_is_a_table() {
    let (registry, _, _, _) = registry();
    let mut tables = Tables::new();

    let id = tables.get_or_insert(&registry, &[]);

    assert!(tables.get(id).unwrap().component_ids().is_empty());
    assert_eq!(tables.len(), 1);
}

#[test]
#[should_panic(expected = "is not registered")]
fn an_unregistered_component_panics() {
    let (registry, _, _, _) = registry();
    let mut tables = Tables::new();

    tables.get_or_insert(&registry, &[StorageId(99)]);
}

/// The column the registry built has to be for the right type — a column
/// of the wrong width would read the next component's bytes.
#[test]
fn the_column_matches_the_component_type() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();
    let id = tables.get_or_insert(&registry, &[health, speed]);
    let table = tables.get_mut(id).unwrap();

    unsafe {
        table.column_mut(health).unwrap().push(Health(120));
        table.column_mut(speed).unwrap().push(Speed(2.5));

        assert_eq!(
            table.column(health).unwrap().get::<Health>(0).unwrap().0,
            120
        );
        assert_eq!(table.column(speed).unwrap().get::<Speed>(0).unwrap().0, 2.5);
    }
}

// -- Moving a row between tables (#891, stage 5a) ---------------------------

use crate::entity::Entity;
use crate::storage::TableRow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts its own drops, so a test can tell a move from a copy.
struct Tracked(Arc<AtomicUsize>);
impl Component for Tracked {}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn a_move_carries_the_shared_components() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();
    let from = tables.get_or_insert(&registry, &[health, speed]);
    let to = tables.get_or_insert(&registry, &[health]);

    let source = tables.get_mut(from).unwrap();
    source.push_entity(entity(7));
    unsafe {
        source.column_mut(health).unwrap().push(Health(120));
        source.column_mut(speed).unwrap().push(Speed(2.5));
    }

    let (landed, displaced) = tables.move_row(from, TableRow(0), to);

    assert_eq!(landed, TableRow(0));
    assert_eq!(displaced, None, "it was the only row");
    let target = tables.get(to).unwrap();
    assert_eq!(target.entities(), &[entity(7)]);
    assert_eq!(
        unsafe { target.column(health).unwrap().get::<Health>(0).unwrap().0 },
        120
    );
    assert!(tables.get(from).unwrap().is_empty());
}

/// 🔴 The test this whole operation exists for. The value was **moved**:
/// there is one copy of it and it lives in the target now. Running the
/// destructor on the way out would be a double free.
#[test]
fn a_moved_value_is_dropped_exactly_once() {
    let mut registry = ComponentRegistry::new();
    let tracked = registry.register_cpu::<Tracked>();
    let health = registry.register_cpu::<Health>();
    let drops = Arc::new(AtomicUsize::new(0));

    let mut tables = Tables::new();
    let from = tables.get_or_insert(&registry, &[tracked]);
    // A different SET, so a different table — and one that still holds
    // `tracked`, so the value is carried rather than destroyed.
    let to = tables.get_or_insert(&registry, &[tracked, health]);

    let source = tables.get_mut(from).unwrap();
    source.push_entity(entity(7));
    unsafe {
        source
            .column_mut(tracked)
            .unwrap()
            .push(Tracked(drops.clone()))
    };

    let (landed, _) = tables.move_row(from, TableRow(0), to);
    assert_eq!(drops.load(Ordering::Relaxed), 0, "moved, not destroyed");
    // The destination is mid-write until `health` is filled; fill it so the
    // table is left in a state its own invariant allows.
    unsafe {
        tables
            .get_mut(to)
            .unwrap()
            .column_mut(health)
            .unwrap()
            .push(Health(1))
    };
    assert_eq!(landed, TableRow(0));

    drop(tables);
    assert_eq!(
        drops.load(Ordering::Relaxed),
        1,
        "destroyed once, at the end"
    );
}

/// A component the destination does not hold is one the entity is losing,
/// so its value is destroyed here and not carried anywhere.
#[test]
fn a_component_the_target_lacks_is_destroyed() {
    let mut registry = ComponentRegistry::new();
    let health = registry.register_cpu::<Health>();
    let tracked = registry.register_cpu::<Tracked>();
    let drops = Arc::new(AtomicUsize::new(0));

    let mut tables = Tables::new();
    let from = tables.get_or_insert(&registry, &[health, tracked]);
    let to = tables.get_or_insert(&registry, &[health]);

    let source = tables.get_mut(from).unwrap();
    source.push_entity(entity(7));
    unsafe {
        source.column_mut(health).unwrap().push(Health(50));
        source
            .column_mut(tracked)
            .unwrap()
            .push(Tracked(drops.clone()));
    }

    tables.move_row(from, TableRow(0), to);

    assert_eq!(drops.load(Ordering::Relaxed), 1, "dropped on the way out");
    assert!(tables.get(to).unwrap().rows_agree());
}

/// An entity gaining a component lands mid-write, and that has to be
/// observable — the value being gained is typed and belongs to the caller.
#[test]
fn gaining_a_component_lands_mid_write() {
    let (registry, health, speed, _) = registry();
    let mut tables = Tables::new();
    let from = tables.get_or_insert(&registry, &[health]);
    let to = tables.get_or_insert(&registry, &[health, speed]);

    let source = tables.get_mut(from).unwrap();
    source.push_entity(entity(7));
    unsafe { source.column_mut(health).unwrap().push(Health(50)) };

    tables.move_row(from, TableRow(0), to);

    let target = tables.get_mut(to).unwrap();
    assert!(!target.rows_agree(), "speed has no value yet");
    unsafe { target.column_mut(speed).unwrap().push(Speed(1.0)) };
    assert!(target.rows_agree());
}

#[test]
fn a_move_reports_the_entity_it_displaced() {
    let (registry, health, _, _) = registry();
    let mut tables = Tables::new();
    let from = tables.get_or_insert(&registry, &[health]);
    let to = tables.get_or_insert(&registry, &[]);

    let source = tables.get_mut(from).unwrap();
    for id in [7u32, 8, 9] {
        source.push_entity(entity(id));
        unsafe { source.column_mut(health).unwrap().push(Health(id)) };
    }

    let (_, displaced) = tables.move_row(from, TableRow(0), to);

    assert_eq!(displaced, Some(entity(9)));
    let source = tables.get(from).unwrap();
    assert_eq!(source.entities(), &[entity(9), entity(8)]);
    assert_eq!(
        unsafe { source.column(health).unwrap().get::<Health>(0).unwrap().0 },
        9,
        "the displaced entity kept its own value"
    );
}

#[test]
#[should_panic(expected = "already in")]
fn moving_a_row_onto_its_own_table_panics() {
    let (registry, health, _, _) = registry();
    let mut tables = Tables::new();
    let id = tables.get_or_insert(&registry, &[health]);
    let table = tables.get_mut(id).unwrap();
    table.push_entity(entity(7));
    unsafe { table.column_mut(health).unwrap().push(Health(1)) };

    tables.move_row(id, TableRow(0), id);
}
