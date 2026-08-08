use super::*;

struct A;
struct B;

fn tracker() -> AccessTracker {
    AccessTracker::new()
}

#[test]
fn multiple_reads_same_type() {
    let tracker = tracker();
    tracker.borrow_read(TypeId::of::<A>());
    tracker.borrow_read(TypeId::of::<A>());
    tracker.release_read(TypeId::of::<A>());
    tracker.release_read(TypeId::of::<A>());
}

#[test]
fn read_and_write_different_types() {
    let tracker = tracker();
    tracker.borrow_read(TypeId::of::<A>());
    tracker.borrow_write(TypeId::of::<B>());
    tracker.release_read(TypeId::of::<A>());
    tracker.release_write(TypeId::of::<B>());
}

#[test]
#[should_panic(expected = "cannot borrow component as mutable: already borrowed")]
fn write_while_read_panics() {
    let tracker = tracker();
    tracker.borrow_read(TypeId::of::<A>());
    tracker.borrow_write(TypeId::of::<A>());
}

#[test]
#[should_panic(expected = "cannot borrow component as immutable: already borrowed as mutable")]
fn read_while_write_panics() {
    let tracker = tracker();
    tracker.borrow_write(TypeId::of::<A>());
    tracker.borrow_read(TypeId::of::<A>());
}

#[test]
#[should_panic(expected = "cannot borrow component as mutable: already borrowed")]
fn double_write_panics() {
    let tracker = tracker();
    tracker.borrow_write(TypeId::of::<A>());
    tracker.borrow_write(TypeId::of::<A>());
}
