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
mod tests;
