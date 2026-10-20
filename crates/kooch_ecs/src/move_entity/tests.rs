//! The funnel: the row and the value move together, or neither does (#891).

use std::any::TypeId;
use std::collections::BTreeSet;

use kooch_core::resource::Resources;

use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::traits::Component;
use crate::component::{ComponentRegistry, StorageId};
use crate::entity::Entity;
use crate::storage::TableRow;

#[derive(Debug, PartialEq)]
struct Health(u32);
impl Component for Health {}

#[derive(Debug, PartialEq)]
struct Speed(f32);
impl Component for Speed {}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn world() -> Resources {
    let mut resources = Resources::new();
    let mut components = ComponentRegistry::new();
    components.register_cpu::<Health>();
    components.register_cpu::<Speed>();
    resources.insert(components);
    resources.insert(ArchetypeRegistry::new());
    resources
}

fn ids(resources: &Resources) -> (StorageId, StorageId) {
    let components = resources.get::<ComponentRegistry>().unwrap();
    (
        components.storage_id(&TypeId::of::<Health>()).unwrap(),
        components.storage_id(&TypeId::of::<Speed>()).unwrap(),
    )
}

fn archetype_of(resources: &mut Resources, types: &[TypeId]) -> ArchetypeId {
    let set: BTreeSet<TypeId> = types.iter().copied().collect();
    resources
        .get_mut::<ArchetypeRegistry>()
        .unwrap()
        .get_or_create(set)
}

/// Files `entity` under the empty archetype, which is where every real spawn starts.
fn spawn(resources: &mut Resources, index: u32) -> Entity {
    let held = entity(index);
    super::move_entity(resources, held, ArchetypeId::EMPTY);
    held
}

/// The whole real gesture: write the value, then move the archetype.
fn give<T: Component>(resources: &mut Resources, held: Entity, value: T) {
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<T>()
        .unwrap()
        .insert(held, value);
    super::gained(resources, held, TypeId::of::<T>());
}

fn row_of(resources: &Resources, held: Entity) -> Option<TableRow> {
    resources.get::<ArchetypeRegistry>().unwrap().row_of(held)
}

#[test]
fn a_gained_value_reaches_its_column() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health(7));

    let (health, _) = ids(&resources);
    let components = resources.get::<ComponentRegistry>().unwrap();
    let (table, row) = components.location(held).unwrap();
    let column = components
        .tables()
        .get(table)
        .unwrap()
        .column(health)
        .unwrap();

    assert_eq!(
        unsafe { column.get::<Health>(row.index()) },
        Some(&Health(7))
    );
    assert_eq!(column.len(), 1, "the inbox was drained, not copied");
}

#[test]
fn the_archetype_hands_out_the_row() {
    let mut resources = world();
    let first = spawn(&mut resources, 1);
    let second = spawn(&mut resources, 2);
    give(&mut resources, first, Health(1));
    give(&mut resources, second, Health(2));

    assert_eq!(row_of(&resources, first), Some(TableRow(0)));
    assert_eq!(row_of(&resources, second), Some(TableRow(1)));
}

/// 🔴 The one that reads another entity's components when it is wrong. A row that leaves drags the
/// last row into the hole, and that entity asked for nothing.
#[test]
fn a_move_fixes_the_displaced() {
    let mut resources = world();
    let (first, second) = (spawn(&mut resources, 1), spawn(&mut resources, 2));
    give(&mut resources, first, Health(1));
    give(&mut resources, second, Health(2));

    // `first` leaves the health-only table, so `second` is dragged into row 0.
    give(&mut resources, first, Speed(9.0));

    assert_eq!(row_of(&resources, second), Some(TableRow(0)));
    let (health, _) = ids(&resources);
    let components = resources.get::<ComponentRegistry>().unwrap();
    let (table, row) = components.location(second).unwrap();
    let column = components
        .tables()
        .get(table)
        .unwrap()
        .column(health)
        .unwrap();
    assert_eq!(
        unsafe { column.get::<Health>(row.index()) },
        Some(&Health(2))
    );
}

#[test]
fn a_move_carries_the_shared_values() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health(5));
    give(&mut resources, held, Speed(2.5));

    let (health, speed) = ids(&resources);
    let components = resources.get::<ComponentRegistry>().unwrap();
    let (table, row) = components.location(held).unwrap();
    let table = components.tables().get(table).unwrap();

    assert_eq!(
        unsafe { table.column(health).unwrap().get::<Health>(row.index()) },
        Some(&Health(5)),
        "the value it already had came along"
    );
    assert_eq!(
        unsafe { table.column(speed).unwrap().get::<Speed>(row.index()) },
        Some(&Speed(2.5))
    );
    assert!(table.rows_agree());
}

#[test]
fn a_lost_component_loses_its_value() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health(5));
    give(&mut resources, held, Speed(2.5));

    super::lost(&mut resources, held, TypeId::of::<Speed>());

    let (_, speed) = ids(&resources);
    let components = resources.get::<ComponentRegistry>().unwrap();
    let (table, _) = components.location(held).unwrap();
    let table = components.tables().get(table).unwrap();
    assert!(
        table.column(speed).is_none(),
        "the column went with the move"
    );
    assert!(table.rows_agree());
}

/// 🔴 The hole in the design if it did not hold: an archetype naming a component with no value
/// would place a row whose column is one short for ever, and the table asserts on the next move.
#[test]
fn a_missing_value_cancels_the_move() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health(5));

    // The archetype gains `Speed` with nothing written for it.
    let both = archetype_of(
        &mut resources,
        &[TypeId::of::<Health>(), TypeId::of::<Speed>()],
    );
    super::move_entity(&mut resources, held, both);

    assert_eq!(row_of(&resources, held), None, "no row was handed out");
    let components = resources.get::<ComponentRegistry>().unwrap();
    assert_eq!(
        components.get_cpu::<Health>().unwrap().get(held),
        Some(&Health(5)),
        "the value it did have went back to the inbox rather than being lost"
    );
}

#[test]
fn a_despawn_frees_the_row() {
    let mut resources = world();
    let (first, second) = (spawn(&mut resources, 1), spawn(&mut resources, 2));
    give(&mut resources, first, Health(1));
    give(&mut resources, second, Health(2));

    super::drop_entity(&mut resources, first);

    assert_eq!(row_of(&resources, second), Some(TableRow(0)));
    let components = resources.get::<ComponentRegistry>().unwrap();
    assert!(components.location(first).is_none());
    let (table, _) = components.location(second).unwrap();
    assert_eq!(components.tables().get(table).unwrap().len(), 1);
}

/// Two archetypes with the same component set share one table — the design requirement that makes
/// a table a property of the set, not of the archetype.
#[test]
fn one_set_gets_one_table() {
    let mut resources = world();
    let first = spawn(&mut resources, 1);
    let second = spawn(&mut resources, 2);
    give(&mut resources, first, Health(1));
    give(&mut resources, second, Health(2));

    let components = resources.get::<ComponentRegistry>().unwrap();
    assert_eq!(
        components.location(first).map(|(table, _)| table),
        components.location(second).map(|(table, _)| table)
    );
    assert_eq!(components.tables().len(), 1);
}

#[test]
fn the_empty_archetype_has_no_table() {
    let mut resources = world();
    spawn(&mut resources, 1);

    let components = resources.get::<ComponentRegistry>().unwrap();
    assert!(
        components.tables().is_empty(),
        "nothing to store, so nothing was built"
    );
}
