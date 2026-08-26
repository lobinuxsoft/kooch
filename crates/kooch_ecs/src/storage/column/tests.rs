use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::Column;

/// Counts its own drops, so a test can assert the column ran them.
///
/// Per-instance rather than a `static`: the harness runs tests in
/// parallel and a shared counter would race.
struct Tracked(Arc<AtomicUsize>);

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// Size 32 and alignment 32. An over-aligned type is where a wrong row
/// stride shows up as a misaligned address and not merely as a wrong
/// value, which is a louder failure and an easier one to read.
#[derive(Debug, PartialEq)]
#[repr(align(32))]
struct Wide(u8);

#[test]
fn it_reads_back_what_it_stored() {
    let mut column = Column::of::<u32>();
    unsafe {
        assert_eq!(column.push(10u32), 0);
        assert_eq!(column.push(20u32), 1);
        assert_eq!(column.push(30u32), 2);

        assert_eq!(column.get::<u32>(0), Some(&10));
        assert_eq!(column.get::<u32>(1), Some(&20));
        assert_eq!(column.get::<u32>(2), Some(&30));
        assert_eq!(column.get::<u32>(3), None);
    }
    assert_eq!(column.len(), 3);
}

#[test]
fn a_write_lands_in_its_row() {
    let mut column = Column::of::<u32>();
    unsafe {
        column.push(10u32);
        column.push(20u32);
        *column.get_mut::<u32>(0).unwrap() = 99;

        assert_eq!(column.get::<u32>(0), Some(&99));
        assert_eq!(column.get::<u32>(1), Some(&20));
    }
}

/// The behaviour the table above depends on: the hole is filled by the
/// last row, which is why the caller has to be told that row moved.
#[test]
fn swap_remove_pulls_the_last() {
    let mut column = Column::of::<u32>();
    unsafe {
        column.push(10u32);
        column.push(20u32);
        column.push(30u32);
    }

    column.swap_remove(0);

    assert_eq!(column.len(), 2);
    unsafe {
        assert_eq!(column.get::<u32>(0), Some(&30));
        assert_eq!(column.get::<u32>(1), Some(&20));
    }
}

/// Removing the last row has nothing to move, and must not copy a row
/// onto itself.
#[test]
fn removing_the_last_moves_nothing() {
    let mut column = Column::of::<u32>();
    unsafe {
        column.push(10u32);
        column.push(20u32);
    }

    column.swap_remove(1);

    assert_eq!(column.len(), 1);
    unsafe { assert_eq!(column.get::<u32>(0), Some(&10)) };
}

#[test]
fn it_drops_what_it_holds() {
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let mut column = Column::of::<Tracked>();
        unsafe {
            column.push(Tracked(drops.clone()));
            column.push(Tracked(drops.clone()));
            column.push(Tracked(drops.clone()));
        }
        assert_eq!(drops.load(Ordering::Relaxed), 0, "nothing dropped yet");
    }
    assert_eq!(drops.load(Ordering::Relaxed), 3);
}

/// Exactly one: the removed item. The row moved into the hole is still
/// live and must not be dropped with it.
#[test]
fn swap_remove_drops_only_one() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut column = Column::of::<Tracked>();
    unsafe {
        column.push(Tracked(drops.clone()));
        column.push(Tracked(drops.clone()));
        column.push(Tracked(drops.clone()));
    }

    column.swap_remove(0);
    assert_eq!(drops.load(Ordering::Relaxed), 1);

    drop(column);
    assert_eq!(drops.load(Ordering::Relaxed), 3, "the other two still ran");
}

/// A marker component is zero-sized, and asking the allocator for zero
/// bytes is undefined. The column has to count without allocating.
#[test]
fn a_zero_sized_item_never_allocates() {
    let mut column = Column::of::<()>();
    unsafe {
        column.push(());
        column.push(());
        column.push(());
        assert_eq!(column.get::<()>(2), Some(&()));
        assert_eq!(column.get::<()>(3), None);
    }
    assert_eq!(column.len(), 3);
    assert_eq!(column.capacity(), 0, "nothing was allocated");
}

/// Growth reallocates, and a stale base pointer or a wrong old layout
/// shows as values that survive the first rows and then do not.
#[test]
fn values_survive_growth() {
    let mut column = Column::of::<u64>();
    for i in 0..200u64 {
        unsafe { column.push(i) };
    }

    assert_eq!(column.len(), 200);
    for i in 0..200usize {
        unsafe { assert_eq!(column.get::<u64>(i), Some(&(i as u64))) };
    }
}

/// Every row of an over-aligned type lands on its own alignment.
///
/// ⚠️ This does **not** guard a padding step, and an earlier version of
/// this comment said it did. Rust already guarantees `size_of` is a
/// multiple of `align_of`, so there is no padding decision to get wrong
/// — replacing the stride with `size_of` was tried and changed nothing.
/// What it does guard is the row arithmetic: a stride off by one fails
/// this and nine other tests with it.
#[test]
fn every_row_is_aligned() {
    let mut column = Column::of::<Wide>();
    for i in 0..10u8 {
        unsafe { column.push(Wide(i)) };
    }

    for i in 0..10usize {
        let value = unsafe { column.get::<Wide>(i).unwrap() };
        let address = std::ptr::from_ref(value) as usize;
        assert_eq!(address % align_of::<Wide>(), 0, "row {i} is misaligned");
        assert_eq!(value.0, i as u8);
    }
}

#[test]
fn clear_drops_and_keeps_the_room() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut column = Column::of::<Tracked>();
    unsafe {
        column.push(Tracked(drops.clone()));
        column.push(Tracked(drops.clone()));
    }
    let capacity = column.capacity();

    column.clear();

    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert!(column.is_empty());
    assert_eq!(column.capacity(), capacity);
}
