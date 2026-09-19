//! The table behind an archetype (#891, stage 5b).

use std::collections::BTreeSet;

use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::ComponentRegistry;
use crate::component::traits::Component;

struct Health(u32);
impl Component for Health {}

struct Speed(f32);
impl Component for Speed {}

fn components() -> ComponentRegistry {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Health>();
    registry.register_cpu::<Speed>();
    registry
}

fn set(types: &[std::any::TypeId]) -> BTreeSet<std::any::TypeId> {
    types.iter().copied().collect()
}

#[test]
fn an_archetype_gets_a_table_for_its_set() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();
    let id = archetypes.get_or_create(set(&[
        std::any::TypeId::of::<Health>(),
        std::any::TypeId::of::<Speed>(),
    ]));

    let table = archetypes.table_of(id, &components).unwrap();
    let table = archetypes.tables().get(table).unwrap();

    assert_eq!(table.component_ids().len(), 2);
    assert!(table.is_empty());
    assert!(table.rows_agree(), "an empty table is a valid one");
}

/// The answer cannot change — an archetype's component set is fixed —
/// so asking twice must not build a second table nobody would notice.
#[test]
fn the_answer_is_stable() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();
    let id = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

    let first = archetypes.table_of(id, &components).unwrap();
    let again = archetypes.table_of(id, &components).unwrap();

    assert_eq!(first, again);
    assert_eq!(archetypes.tables().len(), 1);
}

#[test]
fn different_sets_get_different_tables() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();
    let one = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));
    let two = archetypes.get_or_create(set(&[std::any::TypeId::of::<Speed>()]));

    let a = archetypes.table_of(one, &components).unwrap();
    let b = archetypes.table_of(two, &components).unwrap();

    assert_ne!(a, b);
    assert_eq!(archetypes.tables().len(), 2);
}

/// An entity with no components lives in the empty archetype, and that
/// has to have a table like any other — a column-less one.
#[test]
fn the_empty_archetype_has_a_table() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();

    let table = archetypes
        .table_of(ArchetypeId::EMPTY, &components)
        .unwrap();

    assert!(
        archetypes
            .tables()
            .get(table)
            .unwrap()
            .component_ids()
            .is_empty()
    );
}

#[test]
fn an_unknown_archetype_has_no_table() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();
    let known = archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

    // A different set that was never created.
    let unknown =
        crate::archetype::ArchetypeId::from_components(&set(&[std::any::TypeId::of::<Speed>()]));
    assert_ne!(known, unknown);

    assert_eq!(archetypes.table_of(unknown, &components), None);
}

/// 🔴 Nothing is built until something asks. A table per archetype
/// created eagerly would need the component registry in all 38 places
/// that transition an archetype, none of which cares about storage.
#[test]
fn no_table_exists_until_asked_for() {
    let components = components();
    let mut archetypes = ArchetypeRegistry::new();
    archetypes.get_or_create(set(&[std::any::TypeId::of::<Health>()]));

    assert!(archetypes.tables().is_empty());

    archetypes
        .table_of(ArchetypeId::EMPTY, &components)
        .unwrap();
    assert_eq!(archetypes.tables().len(), 1);
}
