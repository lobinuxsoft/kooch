use super::*;
use crate::coord::{CelestialBodyRef, LocalCoord};

const T: f64 = DEFAULT_REBASE_THRESHOLD_METERS;

#[test]
fn no_rebase_within_threshold() {
    let origin = UniverseCoord::ZERO;
    let player = UniverseCoord::from_dvec3(DVec3::new(T - 1.0, 0.0, 0.0));
    assert_eq!(check_rebase(player, origin, T), RebaseOutcome::Unchanged);
}

#[test]
fn rebase_triggers_past_threshold() {
    let origin = UniverseCoord::ZERO;
    let player = UniverseCoord::from_dvec3(DVec3::new(T + 1.0, 0.0, 0.0));
    match check_rebase(player, origin, T) {
        RebaseOutcome::Rebased { new_origin, delta } => {
            assert_eq!(new_origin, player);
            assert!((delta.x - (T + 1.0)).abs() < 1e-6);
            assert!(delta.y.abs() < 1e-6);
            assert!(delta.z.abs() < 1e-6);
        }
        other => panic!("expected Rebased, got {:?}", other),
    }
}

#[test]
fn rebase_exactly_at_threshold_is_unchanged() {
    // Strict `>` — a delta exactly equal to threshold does NOT
    // rebase. This avoids oscillation when the player wobbles right
    // on the boundary.
    let origin = UniverseCoord::ZERO;
    let player = UniverseCoord::from_dvec3(DVec3::new(T, 0.0, 0.0));
    assert_eq!(check_rebase(player, origin, T), RebaseOutcome::Unchanged);
}

#[test]
fn delta_application_preserves_world_position() {
    // A LocalCoord anchored at the old origin must, after applying
    // the rebase delta, still describe the same absolute world
    // position when resolved against the new origin.
    let old_origin = UniverseCoord::ZERO;
    let new_origin = UniverseCoord::from_dvec3(DVec3::new(2000.0, 0.0, 0.0));
    let world_pos = UniverseCoord::from_dvec3(DVec3::new(2010.0, 0.0, 0.0));

    let local_old = LocalCoord::from_universe(world_pos, old_origin, CelestialBodyRef::NONE);
    assert!((local_old.position.x - 2010.0).abs() < 1e-3);

    let delta = old_origin.delta_to(&new_origin);
    assert!((delta.x - 2000.0).abs() < 1e-6);

    // Shift the local coord by -delta (keeps the world position
    // invariant w.r.t. the new origin).
    let local_new = LocalCoord {
        reference: local_old.reference,
        position: local_old.position - delta.as_vec3(),
    };
    // Now (10, 0, 0): the same world point as before, expressed
    // relative to the new origin.
    assert!((local_new.position.x - 10.0).abs() < 1e-3);

    // Round trip through the new origin recovers the same world.
    let recovered = local_new.to_universe(new_origin);
    assert!((recovered.to_dvec3() - world_pos.to_dvec3()).length() < 1e-3);
}

#[test]
fn diagonal_threshold_uses_euclidean_distance() {
    // (700, 700, 0) has length ~990 m, under T = 1024.
    let origin = UniverseCoord::ZERO;
    let p1 = UniverseCoord::from_dvec3(DVec3::new(700.0, 700.0, 0.0));
    assert_eq!(check_rebase(p1, origin, T), RebaseOutcome::Unchanged);
    // (1000, 1000, 0) has length ~1414 m, over.
    let p2 = UniverseCoord::from_dvec3(DVec3::new(1000.0, 1000.0, 0.0));
    assert!(matches!(
        check_rebase(p2, origin, T),
        RebaseOutcome::Rebased { .. }
    ));
}

#[test]
fn negative_direction_rebases_too() {
    let origin = UniverseCoord::ZERO;
    let player = UniverseCoord::from_dvec3(DVec3::new(-(T + 1.0), 0.0, 0.0));
    match check_rebase(player, origin, T) {
        RebaseOutcome::Rebased { delta, .. } => {
            assert!((delta.x + (T + 1.0)).abs() < 1e-6);
        }
        other => panic!("expected Rebased, got {:?}", other),
    }
}
