//! Rows, and the entity nobody asked about (#891, stage 5c-1).

use std::any::TypeId;
use std::collections::BTreeSet;

use crate::archetype_registry::ArchetypeRegistry;
use crate::component::traits::Component;
use crate::component::{ComponentRegistry, StorageId};
use crate::entity::Entity;
use crate::storage::TableRow;

struct Health(u32);
impl Component for Health {}

struct Speed(f32);
impl Component for Speed {}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn components() -> (ComponentRegistry, StorageId, StorageId) {
    let mut registry = ComponentRegistry::new();
    let health = registry.register_cpu::<Health>();
    let speed = registry.register_cpu::<Speed>();
    (registry, health, speed)
}

fn set(types: &[TypeId]) -> BTreeSet<TypeId> {
    types.iter().copied().collect()
}

/// Places `entity` in the health-only archetype and fills its column,
/// so the table is left in a state its invariant allows.
fn place_with_health(
    archetypes: &mut ArchetypeRegistry,
    components: &ComponentRegistry,
    health: StorageId,
    archetype: crate::archetype::ArchetypeId,
    entity: Entity,
    value: u32,
) -> TableRow {
    let row = archetypes.place(entity, archetype, components).unwrap();
    let table = archetypes.table_of(archetype, components).unwrap();
    unsafe {
        archetypes
            .tables_mut()
            .get_mut(table)
            .unwrap()
            .column_mut(health)
            .unwrap()
            .push(Health(value))
    };
    row
}

#[test]
fn placing_claims_rows_in_order() {
    let (components, health, _) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let arch = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));

    let a = place_with_health(&mut archetypes, &components, health, arch, entity(1), 10);
    let b = place_with_health(&mut archetypes, &components, health, arch, entity(2), 20);

    assert_eq!((a, b), (TableRow(0), TableRow(1)));
    assert_eq!(archetypes.row_of(entity(1)), Some(TableRow(0)));
    assert_eq!(archetypes.row_of(entity(2)), Some(TableRow(1)));
}

/// 🔴 The test this stage exists for. Moving entity 1 out pulls entity 3 into its row — and
/// entity 3 asked for nothing, changed no components and was never mentioned in the call. If
/// nobody fixes its row it reads another entity's values, silently.
#[test]
fn a_move_fixes_the_entity_it_displaced() {
    let (components, health, speed) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let from = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));
    let to = archetypes.get_or_create(set(&[TypeId::of::<Health>(), TypeId::of::<Speed>()]));

    for (id, value) in [(1u32, 10u32), (2, 20), (3, 30)] {
        place_with_health(
            &mut archetypes,
            &components,
            health,
            from,
            entity(id),
            value,
        );
    }

    let landed = archetypes.relocate(entity(1), to, &components).unwrap();
    // The destination gained `speed`, so its column is still unwritten.
    let table = archetypes.table_of(to, &components).unwrap();
    unsafe {
        archetypes
            .tables_mut()
            .get_mut(table)
            .unwrap()
            .column_mut(speed)
            .unwrap()
            .push(Speed(1.0))
    };

    assert_eq!(landed, TableRow(0), "first row of the new table");
    assert_eq!(
        archetypes.row_of(entity(3)),
        Some(TableRow(0)),
        "entity 3 was dragged into the hole and must know it"
    );
    assert_eq!(archetypes.row_of(entity(2)), Some(TableRow(1)), "untouched");

    // And entity 3 now reads ITS value, not entity 1's.
    let source = archetypes.table_of(from, &components).unwrap();
    let column = archetypes
        .tables()
        .get(source)
        .unwrap()
        .column(health)
        .unwrap();
    assert_eq!(unsafe { column.get::<Health>(0).unwrap().0 }, 30);
}

#[test]
fn a_move_carries_the_shared_values() {
    let (components, health, speed) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let from = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));
    let to = archetypes.get_or_create(set(&[TypeId::of::<Health>(), TypeId::of::<Speed>()]));

    place_with_health(&mut archetypes, &components, health, from, entity(1), 77);
    let landed = archetypes.relocate(entity(1), to, &components).unwrap();

    let table = archetypes.table_of(to, &components).unwrap();
    unsafe {
        archetypes
            .tables_mut()
            .get_mut(table)
            .unwrap()
            .column_mut(speed)
            .unwrap()
            .push(Speed(2.0))
    };

    let target = archetypes.tables().get(table).unwrap();
    assert_eq!(
        unsafe {
            target
                .column(health)
                .unwrap()
                .get::<Health>(landed.index())
                .unwrap()
                .0
        },
        77
    );
    assert!(target.rows_agree());
}

/// Eviction has the same hole, so it has the same duty.
#[test]
fn eviction_fixes_the_entity_it_displaced() {
    let (components, health, _) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let arch = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));

    for (id, value) in [(1u32, 10u32), (2, 20), (3, 30)] {
        place_with_health(
            &mut archetypes,
            &components,
            health,
            arch,
            entity(id),
            value,
        );
    }

    assert!(archetypes.evict(entity(1), &components));

    assert_eq!(archetypes.row_of(entity(1)), None);
    assert_eq!(archetypes.row_of(entity(3)), Some(TableRow(0)));
    let table = archetypes.table_of(arch, &components).unwrap();
    let column = archetypes
        .tables()
        .get(table)
        .unwrap()
        .column(health)
        .unwrap();
    assert_eq!(unsafe { column.get::<Health>(0).unwrap().0 }, 30);
}

/// Relocating something that was never placed is a placement, not a
/// failure — the insert path reaches both cases with the same call.
#[test]
fn relocating_an_unplaced_entity_places_it() {
    let (components, health, _) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let arch = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));

    let row = archetypes.relocate(entity(1), arch, &components).unwrap();

    assert_eq!(row, TableRow(0));
    assert_eq!(archetypes.row_of(entity(1)), Some(TableRow(0)));
}

/// 🔴 The entity that MOVES has to learn its new row too — and every other test here happens to
/// move row 0 to row 0, where a stale row is indistinguishable from a correct one. Verified by
/// breaking it: dropping the update fails this and nothing else.
#[test]
fn the_mover_learns_its_new_row() {
    let (components, health, speed) = components();
    let mut archetypes = ArchetypeRegistry::new();
    let from = archetypes.get_or_create(set(&[TypeId::of::<Health>()]));
    let to = archetypes.get_or_create(set(&[TypeId::of::<Health>(), TypeId::of::<Speed>()]));

    // Two entities already living in the destination.
    for id in [5u32, 6] {
        archetypes.place(entity(id), to, &components).unwrap();
        let table = archetypes.table_of(to, &components).unwrap();
        unsafe {
            let tables = archetypes.tables_mut();
            let table = tables.get_mut(table).unwrap();
            table.column_mut(health).unwrap().push(Health(id));
            table.column_mut(speed).unwrap().push(Speed(id as f32));
        }
    }

    place_with_health(&mut archetypes, &components, health, from, entity(1), 99);
    let landed = archetypes.relocate(entity(1), to, &components).unwrap();
    let table = archetypes.table_of(to, &components).unwrap();
    unsafe {
        archetypes
            .tables_mut()
            .get_mut(table)
            .unwrap()
            .column_mut(speed)
            .unwrap()
            .push(Speed(9.0))
    };

    assert_eq!(
        landed,
        TableRow(2),
        "it landed behind the two already there"
    );
    assert_eq!(
        archetypes.row_of(entity(1)),
        Some(TableRow(2)),
        "and the registry recorded it, rather than keeping the old 0"
    );
    let target = archetypes.tables().get(table).unwrap();
    assert_eq!(
        unsafe { target.column(health).unwrap().get::<Health>(2).unwrap().0 },
        99
    );
}

#[test]
fn evicting_something_unplaced_says_so() {
    let (components, _, _) = components();
    let mut archetypes = ArchetypeRegistry::new();

    assert!(!archetypes.evict(entity(9), &components));
}
