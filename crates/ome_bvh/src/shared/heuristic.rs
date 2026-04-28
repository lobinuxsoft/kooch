//! [`should_refit`] — cheap CPU heuristic that picks between rebuild
//! and refit. Intentionally simple: a tighter check would compare
//! volumes too, but the rebuild fallback is safe and fast enough that
//! the simple metric earns its keep until the S7 bench surfaces a
//! workload that justifies tightening it.

use crate::Aabb;

/// Cheap CPU heuristic for the orchestrator: decide between rebuild
/// (full [`super::SharedBvhState::kick`]) and refit (fast
/// [`super::SharedBvhState::kick_refit`]) based on how much each
/// AABB has moved relative to its size.
///
/// Returns `true` (i.e. refit is fine) when **fewer than
/// `change_threshold_pct`** of the AABBs have moved their centre by
/// **more than `move_threshold_ratio`** of their largest extent. Any
/// stretch / shrink that keeps the centre in place is treated as a
/// non-move — this is the cheap proxy; a tighter check would compare
/// volumes too, but the rebuild fallback is safe and fast enough that
/// the simple metric earns its keep.
///
/// Returns `false` (force rebuild) when:
/// - The lengths differ (cardinality changed → refit not viable).
/// - The previous slice is empty (first frame).
/// - The configured percentage of AABBs moved too far.
///
/// Suggested defaults from the PR-5 plan: `move_threshold_ratio =
/// 0.25`, `change_threshold_pct = 10.0`. These are conservative
/// (favour rebuild) — the S7 bench surfaces tighter values once the
/// real workload tells us what "moderate movement" means in practice.
pub fn should_refit(
    prev: &[Aabb],
    curr: &[Aabb],
    move_threshold_ratio: f32,
    change_threshold_pct: f32,
) -> bool {
    if prev.len() != curr.len() || prev.is_empty() {
        return false;
    }
    let mut moved = 0usize;
    for (p, c) in prev.iter().zip(curr.iter()) {
        let extent = p.max - p.min;
        let max_dim = extent.x.max(extent.y).max(extent.z).max(1e-6);
        let centre_delta = (c.center() - p.center()).length();
        if centre_delta > max_dim * move_threshold_ratio {
            moved += 1;
        }
    }
    let pct_moved = moved as f32 / prev.len() as f32 * 100.0;
    pct_moved < change_threshold_pct
}
