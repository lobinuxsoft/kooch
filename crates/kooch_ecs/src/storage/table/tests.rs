use super::{Table, TableRow};
use crate::component::StorageId;
use crate::entity::Entity;
use crate::storage::column::Column;

const HEALTH: StorageId = StorageId(0);
const SPEED: StorageId = StorageId(3);
const ABSENT: StorageId = StorageId(7);

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

/// A table of `u32` health and `f32` speed, with one row per argument.
fn table_of(rows: &[(u32, u32, f32)]) -> Table {
    let mut table = Table::new([(HEALTH, Column::of::<u32>()), (SPEED, Column::of::<f32>())]);
    for &(id, health, speed) in rows {
        table.push_entity(entity(id));
        unsafe {
            table.column_mut(HEALTH).unwrap().push(health);
            table.column_mut(SPEED).unwrap().push(speed);
        }
    }
    assert!(table.rows_agree(), "the fixture built a valid table");
    table
}

fn read(table: &Table, row: usize) -> (Entity, u32, f32) {
    unsafe {
        (
            table.entities()[row],
            *table.column(HEALTH).unwrap().get::<u32>(row).unwrap(),
            *table.column(SPEED).unwrap().get::<f32>(row).unwrap(),
        )
    }
}

#[test]
fn it_holds_a_column_per_component() {
    let table = table_of(&[]);

    assert!(table.column(HEALTH).is_some());
    assert!(table.column(SPEED).is_some());
    assert!(table.column(ABSENT).is_none());
    assert_eq!(table.component_ids(), &[HEALTH, SPEED]);
}

#[test]
#[should_panic(expected = "two columns")]
fn one_component_cannot_have_two_columns() {
    Table::new([(HEALTH, Column::of::<u32>()), (HEALTH, Column::of::<u32>())]);
}

/// The return value the row bookkeeping above this depends on: whoever
/// was last is now sitting in the removed row.
#[test]
fn swap_remove_reports_the_displaced() {
    let mut table = table_of(&[(10, 100, 1.0), (11, 200, 2.0), (12, 300, 3.0)]);

    let displaced = table.swap_remove(TableRow(0));

    assert_eq!(displaced, Some(entity(12)));
    assert_eq!(table.len(), 2);
}

/// Nothing moved, so there is nobody to tell.
#[test]
fn removing_the_last_displaces_nobody() {
    let mut table = table_of(&[(10, 100, 1.0), (11, 200, 2.0)]);

    assert_eq!(table.swap_remove(TableRow(1)), None);
    assert_eq!(table.len(), 1);
    assert_eq!(read(&table, 0), (entity(10), 100, 1.0));
}

/// 🔴 The failure this whole design exists to prevent: a row that means
/// one entity in one column and another entity in the next.
#[test]
fn a_row_means_one_entity_in_every_column() {
    let mut table = table_of(&[(10, 100, 1.0), (11, 200, 2.0), (12, 300, 3.0)]);

    table.swap_remove(TableRow(0));

    // Row 0 is entity 12 now, and it must carry ITS health and ITS speed.
    assert_eq!(read(&table, 0), (entity(12), 300, 3.0));
    assert_eq!(read(&table, 1), (entity(11), 200, 2.0));
}

#[test]
fn removals_can_be_chained() {
    let mut table = table_of(&[
        (10, 100, 1.0),
        (11, 200, 2.0),
        (12, 300, 3.0),
        (13, 400, 4.0),
    ]);

    assert_eq!(table.swap_remove(TableRow(1)), Some(entity(13)));
    assert_eq!(table.swap_remove(TableRow(0)), Some(entity(12)));

    assert_eq!(table.len(), 2);
    assert_eq!(read(&table, 0), (entity(12), 300, 3.0));
    assert_eq!(read(&table, 1), (entity(13), 400, 4.0));
}

/// A row claimed but not filled is exactly the mid-write state, and it
/// has to be observable — otherwise the first thing that notices is a
/// column reading past its own end.
#[test]
fn a_half_written_row_does_not_agree() {
    let mut table = table_of(&[(10, 100, 1.0)]);

    table.push_entity(entity(11));
    assert!(!table.rows_agree(), "one entity, no values");

    unsafe {
        table.column_mut(HEALTH).unwrap().push(200u32);
    }
    assert!(!table.rows_agree(), "still one column short");

    unsafe {
        table.column_mut(SPEED).unwrap().push(2.0f32);
    }
    assert!(table.rows_agree());
}

#[test]
fn rows_are_claimed_in_order() {
    let mut table = Table::new([(HEALTH, Column::of::<u32>())]);

    assert_eq!(table.push_entity(entity(10)), TableRow(0));
    assert_eq!(table.push_entity(entity(11)), TableRow(1));
    assert_eq!(table.entities(), &[entity(10), entity(11)]);
}

#[test]
#[should_panic(expected = "past the end")]
fn removing_a_row_that_is_not_there_panics() {
    let mut table = table_of(&[(10, 100, 1.0)]);
    table.swap_remove(TableRow(5));
}

/// An empty table is a legitimate state — an archetype with no entities
/// in it yet — and must not be a special case anywhere.
#[test]
fn an_empty_table_is_valid() {
    let table = table_of(&[]);

    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
    assert!(table.rows_agree());
    assert!(table.entities().is_empty());
}
