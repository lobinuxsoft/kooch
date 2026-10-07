use super::*;
use glam::Vec3;

fn listed(knots: &[Knot]) -> ReflectValue {
    list_value(knots)
}

fn knots_of(value: &ReflectValue) -> Vec<Knot> {
    list_from(value.clone(), None, "points").expect("a list of knots")
}

/// 🔴 The point of the whole module: an added knot lands ahead of the one before it instead of on
/// the origin, where a reflected list's fixed `element` would always put it.
#[test]
fn an_added_knot_lands_ahead() {
    let before = [
        Knot::at(Vec3::new(5.0, 0.0, 0.0)),
        Knot::at(Vec3::new(6.0, 0.0, 0.0)),
        Knot::default(),
    ];
    let after = knots_of(&placed(&listed(&before)).expect("placed"));

    assert_ne!(after[2].position, Vec3::ZERO, "still on the origin");
    // Ahead of the previous knot, by the step the author was already using.
    assert!(
        after[2].position.distance(before[1].position) > 0.5,
        "{}",
        after[2].position
    );
    assert!(after[2].position.x > before[1].position.x, "went backwards");
}

/// The rhythm is copied, so a path built at five-metre steps keeps them.
#[test]
fn the_spacing_is_copied() {
    let before = [
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(5.0, 0.0, 0.0)),
        Knot::default(),
    ];
    let after = knots_of(&placed(&listed(&before)).expect("placed"));

    assert!(
        (after[2].position.distance(before[1].position) - 5.0).abs() < 0.1,
        "{}",
        after[2].position
    );
}

/// The first knot belongs at the origin it was added at: there is nothing to be ahead of.
#[test]
fn the_first_knot_is_left_alone() {
    assert!(placed(&listed(&[Knot::default()])).is_none());
    assert!(placed(&listed(&[])).is_none());
}

/// A knot the author already moved is not a new one, and moving it would undo their edit.
#[test]
fn a_placed_knot_is_not_moved_again() {
    let settled = [
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(1.0, 0.0, 0.0)),
        Knot::at(Vec3::new(2.0, 0.0, 0.0)),
    ];
    assert!(placed(&listed(&settled)).is_none());
}

/// Only the LAST knot is the added one. A default in the middle is a knot the author put there.
#[test]
fn a_default_in_the_middle_stays() {
    let middle = [
        Knot::at(Vec3::new(1.0, 0.0, 0.0)),
        Knot::default(),
        Knot::at(Vec3::new(3.0, 0.0, 0.0)),
    ];
    assert!(placed(&listed(&middle)).is_none());
}

/// Coincident knots give a zero step, and the fallback keeps the new one off the previous one.
#[test]
fn coincident_knots_still_step() {
    let stalled = [Knot::at(Vec3::ZERO), Knot::at(Vec3::ZERO), Knot::default()];
    let after = knots_of(&placed(&listed(&stalled)).expect("placed"));

    assert!(after[2].position.length() > 0.5, "{}", after[2].position);
    assert!(after[2].position.is_finite());
}
