use super::*;

#[test]
fn a_gap_holds_a_value_between_two() {
    assert_eq!(Order::between(Some(1000), Some(2000)), Some(1500));
    assert_eq!(Order::between(Some(1000), Some(1002)), Some(1001));
}

/// Adjacent values have nothing between them, and saying so is the
/// point: answering `1000` would put the two in an order decided by the
/// sort's stability rather than by this.
#[test]
fn adjacent_values_have_no_room() {
    assert_eq!(Order::between(Some(1000), Some(1001)), None);
    assert_eq!(Order::between(Some(1000), Some(1000)), None);
}

#[test]
fn the_ends_extend_rather_than_collide() {
    assert_eq!(Order::between(None, None), Some(Order::STEP));
    assert_eq!(Order::between(Some(5000), None), Some(6000));
    assert_eq!(Order::between(None, Some(1000)), Some(500));
    assert_eq!(Order::between(None, Some(0)), None, "no room below zero");
    assert_eq!(Order::between(Some(u32::MAX), None), None, "no room above");
}

/// Repeatedly dropping between the same pair narrows the gap and then
/// says so, rather than silently placing two siblings at one value.
#[test]
fn a_gap_runs_out_and_reports_it() {
    let low = Order::STEP;
    let mut high = Order::STEP * 2;
    let mut inserts = 0;
    while let Some(mid) = Order::between(Some(low), Some(high)) {
        high = mid;
        inserts += 1;
        assert!(inserts < 64, "the gap never closed");
    }
    assert!(
        (9..=11).contains(&inserts),
        "a 1000-wide gap took {inserts} insertions, not about ten",
    );
}

#[test]
fn renumbering_spaces_a_group_out() {
    assert_eq!(Order::spaced(3).collect::<Vec<_>>(), vec![1000, 2000, 3000]);
    assert!(Order::spaced(0).next().is_none());
}
