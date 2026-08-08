use super::*;
use crate::coord::DEFAULT_REBASE_THRESHOLD_METERS;
use glam::DVec3;

#[test]
fn default_is_zero() {
    let origin = ActiveOrigin::default();
    assert_eq!(origin, ActiveOrigin::ZERO);
    assert_eq!(origin.coord(), UniverseCoord::ZERO);
}

#[test]
fn set_updates_coord() {
    let mut origin = ActiveOrigin::default();
    let new_coord = UniverseCoord::from_dvec3(DVec3::new(5000.0, 0.0, 0.0));
    origin.set(new_coord);
    assert_eq!(origin.coord(), new_coord);
}

#[test]
fn evaluate_rebase_delegates_to_check_rebase() {
    let origin = ActiveOrigin::ZERO;
    // Player within threshold → unchanged.
    let near = UniverseCoord::from_dvec3(DVec3::new(100.0, 0.0, 0.0));
    assert_eq!(
        origin.evaluate_rebase(near, DEFAULT_REBASE_THRESHOLD_METERS),
        super::super::RebaseOutcome::Unchanged
    );
    // Player past threshold → rebased.
    let far =
        UniverseCoord::from_dvec3(DVec3::new(DEFAULT_REBASE_THRESHOLD_METERS + 1.0, 0.0, 0.0));
    assert!(matches!(
        origin.evaluate_rebase(far, DEFAULT_REBASE_THRESHOLD_METERS),
        super::super::RebaseOutcome::Rebased { .. }
    ));
}
