//! Origin rebasing — keep the active local origin near the player so
//! the camera-relative f32 pipeline never sees positions large enough
//! to lose precision.
//!
//! The function in this module is **decision + delta only**: it reads
//! the player's [`UniverseCoord`] and the current origin, and returns
//! whether to rebase plus the world-space delta to apply. The actual
//! application to ECS entities (shifting `LocalCoord`s, `Transform`s,
//! GPU resident positions) lives in `kooch_ecs` / `kooch_render` so this
//! crate stays agnostic of the ECS layer.
//!
//! See `feedback_planet_scale_gpu_driven` (memory) and issue #50.

use glam::DVec3;

use crate::coord::UniverseCoord;

/// Default threshold (in meters) past which the active origin rebases
/// toward the player. Set at 1024 m — well below the f32 precision
/// cliff (~5 km), with margin for a fast-travelling player to cross
/// the threshold mid-frame without the rebase reaction lagging visibly.
pub const DEFAULT_REBASE_THRESHOLD_METERS: f64 = 1024.0;

/// Outcome of a rebase check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RebaseOutcome {
    /// Player is still close enough to the origin — nothing to do.
    Unchanged,
    /// Player has drifted past the threshold. The active origin should
    /// be set to `new_origin`, and every position currently anchored to
    /// the old origin must be shifted by `-delta` (subtract `delta` from
    /// `LocalCoord.position` / GPU-resident world positions) to keep
    /// the world invariant after the swap.
    Rebased {
        new_origin: UniverseCoord,
        /// World-space displacement (in meters) from old origin to new
        /// origin.
        delta: DVec3,
    },
}

/// Decide whether the active origin should rebase toward the player.
///
/// Returns [`RebaseOutcome::Rebased`] when `|player - current_origin| > threshold`.
/// The caller stores the new origin and shifts in-flight positions held
/// in the old origin's frame.
///
/// The check uses strict `>`, not `>=` — a player exactly on the
/// threshold does not trigger a rebase. This avoids spurious rebases
/// when the player oscillates around an axis-aligned boundary.
pub fn check_rebase(
    player: UniverseCoord,
    current_origin: UniverseCoord,
    threshold_meters: f64,
) -> RebaseOutcome {
    let delta = current_origin.delta_to(&player);
    if delta.length() > threshold_meters {
        RebaseOutcome::Rebased {
            new_origin: player,
            delta,
        }
    } else {
        RebaseOutcome::Unchanged
    }
}

#[cfg(test)]
mod tests {
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
}
