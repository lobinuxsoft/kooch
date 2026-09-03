use super::*;

/// A wall lets go and a step does not — the whole reason the two are
/// told apart, since their contact normals are identical.
#[test]
fn only_a_wall_lets_go() {
    assert!(Footing::Ground.holds());
    assert!(Footing::Step.holds());
    assert!(!Footing::Wall.holds());
}

/// A step is something you are getting over, not stood on, so a jump
/// must not think it has a floor under it halfway up one.
#[test]
fn a_step_is_not_standing() {
    assert!(Footing::Ground.stands());
    assert!(!Footing::Step.stands());
    assert!(!Footing::Wall.stands());
}
