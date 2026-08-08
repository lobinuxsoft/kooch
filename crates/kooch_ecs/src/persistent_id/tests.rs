use super::*;

#[test]
fn zero_is_not_a_valid_guid() {
    assert!(EntityGuid::new(0).is_none());
    assert!(EntityGuid::new(1).is_some());
}

/// The niche is the whole reason for `NonZeroU64`; if this regresses,
/// every `Option<EntityGuid>` field silently doubles.
#[test]
fn an_optional_guid_costs_nothing_extra() {
    assert_eq!(
        size_of::<Option<EntityGuid>>(),
        size_of::<EntityGuid>(),
        "Option<EntityGuid> must stay 8 bytes",
    );
}

#[test]
fn ids_are_sequential_and_never_zero() {
    let mut alloc = PersistentIdAllocator::new();
    let first = alloc.allocate();
    let second = alloc.allocate();
    assert_eq!(first.get(), 1);
    assert_eq!(second.get(), 2);
}

#[test]
fn observing_an_id_stops_it_being_reissued() {
    let mut alloc = PersistentIdAllocator::new();
    alloc.observe(EntityGuid::new(42).unwrap());
    assert_eq!(alloc.allocate().get(), 43);
}

/// A scene file is not trusted to move the watermark backwards. Loading
/// one that claims a lower value than ids already live would reissue
/// them, and the aliasing would only show up as two entities answering
/// to one reference.
#[test]
fn resuming_never_moves_the_watermark_backwards() {
    let mut alloc = PersistentIdAllocator::new();
    alloc.observe(EntityGuid::new(100).unwrap());
    alloc.resume_from(5);
    assert_eq!(alloc.allocate().get(), 101);
}

#[test]
fn a_watermark_round_trips_through_a_fresh_allocator() {
    let mut alloc = PersistentIdAllocator::new();
    alloc.allocate();
    alloc.allocate();

    let mut reloaded = PersistentIdAllocator::new();
    reloaded.resume_from(alloc.watermark());
    assert_eq!(reloaded.allocate().get(), 3, "ids must not be reissued");
}
