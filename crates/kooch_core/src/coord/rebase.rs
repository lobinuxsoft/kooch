//! Origin rebasing — keep the active local origin near the player so the camera-relative f32
//! pipeline never sees positions large enough to lose precision.

use glam::DVec3;

use crate::coord::UniverseCoord;

/// Default threshold (in meters) past which the active origin rebases toward the player. Set at
/// 1024 m — well below the f32 precision cliff (~5 km), with margin for a fast-travelling player to
/// cross the threshold mid-frame without the rebase reaction lagging visibly.
pub const DEFAULT_REBASE_THRESHOLD_METERS: f64 = 1024.0;

/// Outcome of a rebase check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RebaseOutcome {
    /// Player is still close enough to the origin — nothing to do.
    Unchanged,
    /// Player has drifted past the threshold.
    Rebased {
        new_origin: UniverseCoord,
        /// World-space displacement (in meters) from old origin to new
        /// origin.
        delta: DVec3,
    },
}

/// Decide whether the active origin should rebase toward the player.
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
mod tests;
