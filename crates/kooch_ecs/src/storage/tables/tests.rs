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
