use super::*;

#[test]
fn one_member_is_exactly_its_own_position() {
    let position = Vec3::new(3.0, 4.0, 5.0);
    assert_eq!(weighted_centre(&[(position, 1.0)]), Some(position));
}

/// The single-target case must not shift when the weight is not 1.
#[test]
fn one_member_ignores_its_weight() {
    let position = Vec3::new(3.0, 4.0, 5.0);
    assert_eq!(weighted_centre(&[(position, 7.5)]), Some(position));
}

#[test]
fn two_equal_members_meet_in_the_middle() {
    let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(10.0, 0.0, 0.0), 1.0)]);
    assert_eq!(centre, Some(Vec3::new(5.0, 0.0, 0.0)));
}

#[test]
fn weights_are_relative_not_absolute() {
    let members = [(Vec3::ZERO, 1.0), (Vec3::new(9.0, 0.0, 0.0), 2.0)];
    let scaled = [(Vec3::ZERO, 10.0), (Vec3::new(9.0, 0.0, 0.0), 20.0)];
    assert_eq!(weighted_centre(&members), weighted_centre(&scaled));
}

#[test]
fn a_heavier_member_pulls_the_centre_towards_itself() {
    let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(9.0, 0.0, 0.0), 2.0)])
        .expect("two positive weights");
    assert!(
        centre.x > 4.5,
        "the double-weighted member should pull past the midpoint; got {centre:?}"
    );
    assert_eq!(centre, Vec3::new(6.0, 0.0, 0.0));
}

#[test]
fn an_empty_group_has_no_centre() {
    assert_eq!(weighted_centre(&[]), None);
}

/// Zero weight is "ignore me", and a group of only those is empty.
#[test]
fn a_group_of_zero_weights_has_no_centre() {
    assert_eq!(
        weighted_centre(&[(Vec3::ZERO, 0.0), (Vec3::ONE, 0.0)]),
        None
    );
}

/// A negative weight would drag the centre away from the subject and
/// could cancel the total to zero, which reads as "no target".
#[test]
fn a_negative_weight_is_treated_as_zero() {
    let centre = weighted_centre(&[(Vec3::ZERO, 1.0), (Vec3::new(10.0, 0.0, 0.0), -5.0)]);
    assert_eq!(
        centre,
        Some(Vec3::ZERO),
        "a negative weight must not move the centre, nor void the group"
    );
}
