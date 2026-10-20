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

#[derive(Debug, Default, PartialEq, crate::Reflect)]
struct Health {
    hp: u32,
}
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
    components.register_cpu_reflected::<Health>();
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
    give(&mut resources, held, Health { hp: 7 });

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
        Some(&Health { hp: 7 })
    );
    assert_eq!(column.len(), 1, "the inbox was drained, not copied");
}

#[test]
fn the_archetype_hands_out_the_row() {
    let mut resources = world();
    let first = spawn(&mut resources, 1);
    let second = spawn(&mut resources, 2);
    give(&mut resources, first, Health { hp: 1 });
    give(&mut resources, second, Health { hp: 2 });

    assert_eq!(row_of(&resources, first), Some(TableRow(0)));
    assert_eq!(row_of(&resources, second), Some(TableRow(1)));
}

/// 🔴 The one that reads another entity's components when it is wrong. A row that leaves drags the
/// last row into the hole, and that entity asked for nothing.
#[test]
fn a_move_fixes_the_displaced() {
    let mut resources = world();
    let (first, second) = (spawn(&mut resources, 1), spawn(&mut resources, 2));
    give(&mut resources, first, Health { hp: 1 });
    give(&mut resources, second, Health { hp: 2 });

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
        Some(&Health { hp: 2 })
    );
}

#[test]
fn a_move_carries_the_shared_values() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health { hp: 5 });
    give(&mut resources, held, Speed(2.5));

    let (health, speed) = ids(&resources);
    let components = resources.get::<ComponentRegistry>().unwrap();
    let (table, row) = components.location(held).unwrap();
    let table = components.tables().get(table).unwrap();

    assert_eq!(
        unsafe { table.column(health).unwrap().get::<Health>(row.index()) },
        Some(&Health { hp: 5 }),
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
    give(&mut resources, held, Health { hp: 5 });
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
    give(&mut resources, held, Health { hp: 5 });

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
        Some(&Health { hp: 5 }),
        "the value it did have went back to the inbox rather than being lost"
    );
}

#[test]
fn a_despawn_frees_the_row() {
    let mut resources = world();
    let (first, second) = (spawn(&mut resources, 1), spawn(&mut resources, 2));
    give(&mut resources, first, Health { hp: 1 });
    give(&mut resources, second, Health { hp: 2 });

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
    give(&mut resources, first, Health { hp: 1 });
    give(&mut resources, second, Health { hp: 2 });

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

/// 🔴 The invariant the port rests on, and the one nothing else can see breaking.
#[test]
fn the_funnel_keeps_the_rows_equal() {
    let mut resources = world();
    // 🔴 Every entity settles **first**, so the moves below leave from the middle of the table and
    // actually drag someone into the hole. Moving the last row displaces nobody, and a scenario
    // built that way passes with the refiling deleted.
    for index in 1..6 {
        let held = spawn(&mut resources, index);
        give(&mut resources, held, Health { hp: index });
    }
    give(&mut resources, entity(2), Speed(2.0));
    give(&mut resources, entity(1), Speed(1.0));
    super::drop_entity(&mut resources, entity(4));
    super::lost(&mut resources, entity(2), TypeId::of::<Speed>());

    let components = resources.get::<ComponentRegistry>().unwrap();
    let archetypes = resources.get::<ArchetypeRegistry>().unwrap();
    assert!(super::rows_agree(components, archetypes));
}

/// And it catches the bypass: filing an entity by hand leaves the two rows disagreeing, which is
/// exactly the state that makes a query and `get_cpu` answer differently.
#[test]
fn filing_by_hand_breaks_them() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health { hp: 1 });

    let both = archetype_of(
        &mut resources,
        &[TypeId::of::<Health>(), TypeId::of::<Speed>()],
    );
    resources
        .get_mut::<ArchetypeRegistry>()
        .unwrap()
        .register_at(held, both, None);

    let components = resources.get::<ComponentRegistry>().unwrap();
    let archetypes = resources.get::<ArchetypeRegistry>().unwrap();
    assert!(
        !super::rows_agree(components, archetypes),
        "the check is blind"
    );
}

/// A reused index must not inherit the row of whoever held the slot before it.
#[test]
fn a_reused_index_starts_clean() {
    let mut resources = world();
    let first = spawn(&mut resources, 1);
    give(&mut resources, first, Health { hp: 1 });
    super::drop_entity(&mut resources, first);

    let reused = Entity::new(1, 1);
    let components = resources.get::<ComponentRegistry>().unwrap();
    assert_eq!(components.location(reused), None);
    assert_eq!(
        components.get_cpu::<Health>().unwrap().get(reused),
        None,
        "it read the generation before it"
    );
}

/// 🔴 What the editor's mirror does on **every** snapshot: default the component, then write the
/// fields over it. Twice in a row has to leave one value, not two.
#[test]
fn a_reapplied_snapshot_keeps_one() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health { hp: 7 });
    let health = TypeId::of::<Health>();

    for pass in 0..3u32 {
        let components = resources.get_mut::<ComponentRegistry>().unwrap();
        components.insert_default_reflected(&health, held);
        components
            .reflect_set_field(&health, held, "hp", crate::ReflectValue::U32(pass + 10))
            .expect("the field takes the write");

        let view = components.get_cpu::<Health>().unwrap();
        assert_eq!(view.iter().count(), 1, "pass {pass} left two values");
        assert_eq!(
            view.get(held).map(|h| h.hp),
            Some(pass + 10),
            "pass {pass} read a default instead of the value written"
        );
        assert!(
            components.no_stale_inbox(),
            "pass {pass} parked a second value"
        );
    }
}

/// 🔴 A default written over a value that already lives in a column lands in the inbox, and then
/// the component has TWO values: the column's, and a default nobody asked for. `iter` yields the
/// entity twice and whichever comes last wins — which is how every transform in a loaded scene
/// ended up at the identity.
#[test]
fn a_second_default_does_not_double() {
    let mut resources = world();
    let held = spawn(&mut resources, 1);
    give(&mut resources, held, Health { hp: 7 });

    let components = resources.get_mut::<ComponentRegistry>().unwrap();
    components.insert_default_reflected(&TypeId::of::<Health>(), held);

    let view = components.get_cpu::<Health>().unwrap();
    assert_eq!(view.len(), 1, "the component has two values");
    assert_eq!(view.iter().count(), 1, "the walk sees the entity twice");
}
