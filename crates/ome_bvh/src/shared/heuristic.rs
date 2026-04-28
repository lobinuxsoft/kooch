//! [`should_refit`] — cheap CPU heuristic that picks between rebuild
//! and refit. Intentionally simple: a tighter check would compare
//! volumes too, but the rebuild fallback is safe and fast enough that
//! the simple metric earns its keep until the S7 bench surfaces a
//! workload that justifies tightening it.
//!
//! [`SharedBvhState::kick_auto`] composes this heuristic with the
//! orchestrator's lifecycle (`kick` / `kick_refit`) and lives here too:
//! the policy lives next to the predicate it consults, and `state.rs`
//! stays bounded under the no-monolithic threshold.

use crate::leaf::LeafAabb;
use crate::Aabb;

use super::pending::BuildToken;
use super::state::SharedBvhState;

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

impl SharedBvhState {
    /// Unified rebuild-vs-refit lifecycle entry point. Internally picks
    /// between [`SharedBvhState::kick`] and [`SharedBvhState::kick_refit`]
    /// based on the [`should_refit`] heuristic over the previously-
    /// mirrored leaf AABBs. Suppressed under the same conditions as the
    /// underlying methods (pending in flight, hash unchanged, cardinality
    /// drift).
    ///
    /// Decision matrix:
    ///
    /// - First build (`current_n() == 0` / no CPU mirror) → rebuild.
    /// - Cardinality changed → rebuild.
    /// - `should_refit(prev_aabbs, new_aabbs, ...)` says "yes" → refit.
    /// - Otherwise → rebuild.
    ///
    /// `move_threshold_ratio` and `change_threshold_pct` forward
    /// directly to [`should_refit`]. PR-5 plan defaults are `0.25` and
    /// `10.0`; tighter values land via the S7 bench results.
    ///
    /// Lifetime counters [`SharedBvhState::builds_kicked`] /
    /// [`SharedBvhState::refits_kicked`] reflect the chosen path so
    /// callers can introspect "what did the orchestrator just decide"
    /// without tracking it themselves.
    pub fn kick_auto(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        scene_hash: u64,
        move_threshold_ratio: f32,
        change_threshold_pct: f32,
    ) -> Option<BuildToken<'_>> {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        // Decide rebuild vs refit BEFORE mutating any state. The two
        // public accessors return immutable borrows that drop at the
        // end of this block, so the subsequent `&mut self` calls don't
        // collide. Refit is viable only when there's a previous CPU
        // mirror to compare against and the cardinality matches —
        // both gates are re-checked downstream anyway, so this is a
        // pure suggestion.
        let prefer_refit = {
            match (self.current_cpu_bvh(), self.current_cpu_leaf_aabbs()) {
                (Some(bvh), Some(prev_leaves)) if bvh.leaf_count() == items.len() => {
                    let prev: Vec<Aabb> = prev_leaves
                        .iter()
                        .map(|la| Aabb::new(la.aabb_min.into(), la.aabb_max.into()))
                        .collect();
                    let curr: Vec<Aabb> = items.iter().map(|(_, a)| *a).collect();
                    should_refit(&prev, &curr, move_threshold_ratio, change_threshold_pct)
                }
                _ => false,
            }
        };

        if prefer_refit {
            self.kick_refit(device, queue, items, leaf_aabbs, scene_hash)
        } else {
            self.kick(device, queue, items, leaf_aabbs, scene_hash)
        }
    }
}
