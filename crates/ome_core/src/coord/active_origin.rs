//! [`ActiveOrigin`] — the universe coordinate that defines `(0, 0, 0)` of
//! the simulation frame the rest of the engine operates in.
//!
//! Every entity's `GlobalTransform` (a `Mat4` of f32) is relative to
//! `ActiveOrigin`. As the player travels across sectors the rebase
//! system (built on top of [`super::rebase::check_rebase`]) updates
//! this resource and shifts entity positions to keep them near the
//! origin — preserving f32 precision in the camera-relative pipeline
//! regardless of how far the player has travelled in the universe.
//!
//! Until origin rebasing is wired into the ECS lifecycle, this resource
//! sits at [`UniverseCoord::ZERO`] and the engine behaves identically
//! to the pre-coords pipeline.

use crate::coord::UniverseCoord;

/// Universe coordinate the simulation frame is anchored at.
///
/// Inserted as a resource by [`OriginPlugin`]. Read by render pipelines
/// (raymarch, mesh, sky) when they need the absolute universe position
/// of camera / scene entities — for logging, debug HUDs, or per-pipeline
/// world-relative effects.
///
/// Mutated only by the rebase system (filed as a follow-up to #50).
/// Manual mutation is allowed but will not move existing entities — use
/// [`Self::rebase_to`] to perform the same checked-and-tracked update
/// the rebase system would do.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActiveOrigin {
    coord: UniverseCoord,
}

impl ActiveOrigin {
    pub const ZERO: Self = Self {
        coord: UniverseCoord::ZERO,
    };

    pub const fn new(coord: UniverseCoord) -> Self {
        Self { coord }
    }

    /// The universe coordinate currently treated as `(0, 0, 0)` of the
    /// simulation frame.
    pub fn coord(&self) -> UniverseCoord {
        self.coord
    }

    /// Replace the active origin. Does **not** shift any entities — the
    /// caller is responsible for issuing the matching delta application
    /// to `Query<&mut GlobalTransform>` (or equivalent) to keep world
    /// positions invariant. Prefer the rebase system over manual sets.
    pub fn set(&mut self, coord: UniverseCoord) {
        self.coord = coord;
    }

    /// Convenience: returns the rebase outcome for a player at
    /// `player_position` against this origin and `threshold`. Mirrors
    /// [`super::check_rebase`] but reads `self.coord` for the current
    /// origin so callers don't have to thread it.
    pub fn evaluate_rebase(
        &self,
        player_position: UniverseCoord,
        threshold_meters: f64,
    ) -> super::RebaseOutcome {
        super::check_rebase(player_position, self.coord, threshold_meters)
    }
}

#[cfg(test)]
mod tests {
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
        let far = UniverseCoord::from_dvec3(DVec3::new(
            DEFAULT_REBASE_THRESHOLD_METERS + 1.0,
            0.0,
            0.0,
        ));
        assert!(matches!(
            origin.evaluate_rebase(far, DEFAULT_REBASE_THRESHOLD_METERS),
            super::super::RebaseOutcome::Rebased { .. }
        ));
    }
}
