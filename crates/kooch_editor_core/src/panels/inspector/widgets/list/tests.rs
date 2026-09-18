//! Test code for `list`, in its own file.

use kooch_ecs::reflect::ReflectValue;

use super::{Change, applied};

fn items(values: &[u32]) -> Vec<ReflectValue> {
    values.iter().copied().map(ReflectValue::U32).collect()
}

fn values(list: ReflectValue) -> Vec<u32> {
    let ReflectValue::List { items, .. } = list else {
        panic!("not a list");
    };
    items
        .into_iter()
        .map(|item| match item {
            ReflectValue::U32(v) => v,
            other => panic!("{other:?}"),
        })
        .collect()
}

/// Moving an effect up runs it earlier — the order is the whole point of a stack.
#[test]
fn up_swaps_with_the_one_before() {
    let list = applied(&items(&[1, 2, 3]), &ReflectValue::U32(0), Change::Up(2));
    assert_eq!(values(list), [1, 3, 2]);
}

#[test]
fn down_swaps_with_the_one_after() {
    let list = applied(&items(&[1, 2, 3]), &ReflectValue::U32(0), Change::Down(0));
    assert_eq!(values(list), [2, 1, 3]);
}

/// A new item is the element template, which is what makes an empty list addable.
#[test]
fn add_appends_the_template() {
    let list = applied(&[], &ReflectValue::U32(7), Change::Add);
    assert_eq!(values(list), [7]);
}

#[test]
fn remove_takes_one_out() {
    let list = applied(&items(&[1, 2, 3]), &ReflectValue::U32(0), Change::Remove(1));
    assert_eq!(values(list), [1, 3]);
}
