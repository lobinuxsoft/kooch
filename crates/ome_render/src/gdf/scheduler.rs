//! Round-robin GDF cascade update scheduler. PR-5 of epic #370.
//!
//! Cascade `c` redispatches every `2^c` frames in steady state — six
//! cascades collapse to one populate dispatch per frame on average:
//! 1 + 1/2 + 1/4 + 1/8 + 1/16 + 1/32 = 1.96. Plus on-demand triggers
//! when a chunk inside a cascade's AABB is dirtied or when the
//! camera drifts more than 4 voxels from the last cascade origin.
//!
//! State is plain-old-data: `frame_idx: u64`, per-cascade dirty
//! bit (whether ANY chunk has been dirtied since the cascade last
//! ran), and `last_centre[6]` snapshot of the camera position at
//! each cascade's most recent dispatch. Total: 64 + 6 + 6×12 = 142 B.
//! Trivial CPU-side; per-chunk granularity is reserved for a future
//! PR once chunk count outgrows the bitset budget.

use glam::Vec3;

use super::uniforms::{CASCADE_COUNT, CASCADE_VOXEL_SIZES, GdfUniforms};

/// Camera-drift threshold in voxel units. Past this, the cascade is
/// re-snapped even if it's not on the round-robin schedule. 4 voxels
/// = 1.0 m for cascade 0 (voxel pitch 0.25 m); enough headroom that
/// fractional camera motion doesn't trigger every frame, but small
/// enough that the cascade origin tracks the camera before voxel
/// quantisation surfaces as a rendering artefact.
pub const CAMERA_DRIFT_VOXELS: f32 = 4.0;

/// Round-robin scheduler driving per-cascade populate dispatches.
///
/// Build with [`GdfScheduler::new`], call
/// [`GdfScheduler::cascades_to_update`] once per frame to get the
/// cascade indices to dispatch this frame. Mark dirty chunks via
/// [`GdfScheduler::mark_chunk_dirty`] so on-demand triggers fire when
/// content updates can't wait for the steady-state cadence.
#[derive(Debug, Clone)]
pub struct GdfScheduler {
    /// Monotonic frame counter incremented inside `cascades_to_update`.
    /// Owned here (not the renderer) so the scheduler is the single
    /// source of truth for "current frame" — debug overlays and
    /// tests don't need a parallel counter.
    frame_idx: u64,
    /// Per-cascade dirty flag: at least one chunk has been dirtied
    /// since the cascade last dispatched. Cleared on dispatch.
    cascades_dirty: [bool; CASCADE_COUNT],
    /// Camera position at the cascade's most recent dispatch. Used
    /// to force a re-snap when the camera has drifted past
    /// `CAMERA_DRIFT_VOXELS * voxel_size` since then.
    last_centre: [Vec3; CASCADE_COUNT],
    /// Whether each cascade has dispatched at least once. First-frame
    /// stagger: cascade `c` skips its first scheduled run until
    /// frame `c` so we don't spike with all six dispatches at frame 0.
    bootstrapped: [bool; CASCADE_COUNT],
}

impl Default for GdfScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl GdfScheduler {
    pub fn new() -> Self {
        Self {
            frame_idx: 0,
            cascades_dirty: [false; CASCADE_COUNT],
            last_centre: [Vec3::ZERO; CASCADE_COUNT],
            bootstrapped: [false; CASCADE_COUNT],
        }
    }

    /// Compute the set of cascade indices that need a populate
    /// dispatch this frame. Walks finest → coarsest. Three steady-
    /// state triggers + a first-run bootstrap:
    ///
    /// 1. **Bootstrap:** cascade `c` runs once on frame `c`. Avoids
    ///    the cold-start spike where all six dispatches collapse to
    ///    frame 0.
    /// 2. **Schedule:** post-bootstrap, `frame_idx % 2^c == 0`
    ///    (cascade 0 every frame, cascade 5 every 32 frames).
    /// 3. **Dirty:** any chunk has been dirtied since the cascade
    ///    last ran (via [`Self::mark_chunk_dirty`]).
    /// 4. **Drift:** camera moved more than `CAMERA_DRIFT_VOXELS *
    ///    voxel_size[c]` since the cascade's last dispatch.
    ///
    /// Mutates internal state — clears the dirty flag and updates
    /// `last_centre` / `bootstrapped` for dispatched cascades.
    /// Increments `frame_idx` once per call.
    pub fn cascades_to_update(&mut self, camera: Vec3) -> Vec<u32> {
        let mut to_update: Vec<u32> = Vec::with_capacity(CASCADE_COUNT);
        let frame = self.frame_idx;
        for c in 0..CASCADE_COUNT {
            let bootstrap = !self.bootstrapped[c] && frame == c as u64;
            // Schedule triggers only AFTER bootstrap. Pre-bootstrap,
            // a cascade is dormant — it has no `last_centre` to
            // compare against and the frame index might still be
            // counting down to its bootstrap slot.
            let on_schedule = self.bootstrapped[c] && frame % (1u64 << c) == 0;
            let dirty = self.bootstrapped[c] && self.cascades_dirty[c];
            let drift_threshold = CAMERA_DRIFT_VOXELS * CASCADE_VOXEL_SIZES[c];
            let camera_drift = self.bootstrapped[c]
                && (camera - self.last_centre[c]).length() > drift_threshold;
            if bootstrap || on_schedule || dirty || camera_drift {
                to_update.push(c as u32);
                self.cascades_dirty[c] = false;
                self.last_centre[c] = camera;
                self.bootstrapped[c] = true;
            }
        }
        self.frame_idx = self.frame_idx.wrapping_add(1);
        to_update
    }

    /// Mark every cascade as dirty. Called when a chunk is inserted,
    /// removed, or refit so the next pass picks up the change without
    /// waiting for round-robin. Per-cascade chunk-AABB intersection
    /// (only mark cascades whose AABB overlaps the dirty chunk's
    /// AABB) is a future optimisation — for now the scheduler is
    /// conservative.
    pub fn mark_chunk_dirty(&mut self, _chunk_idx: u32) {
        for c in 0..CASCADE_COUNT {
            self.cascades_dirty[c] = true;
        }
    }

    /// Mirror of [`Self::mark_chunk_dirty`] for callers that don't
    /// have a chunk index handy (e.g. the streaming bridge knows
    /// "something changed" without yet having allocated the chunk
    /// entry).
    pub fn mark_all_dirty(&mut self) {
        self.cascades_dirty = [true; CASCADE_COUNT];
    }

    pub fn frame_idx(&self) -> u64 {
        self.frame_idx
    }
}

/// Rust mirror of `raymarch_gdf_sample.wgsl::pick_cascade`. Walks
/// finest → coarsest, returns the first cascade whose AABB contains
/// `p_world` AND whose voxel pitch is at least `cone_radius`. `None`
/// when no cascade qualified — the shader's sentinel `6u`.
///
/// Used by `tests/gdf_multi_cascade.rs::cascade_selection_picks_finest_for_close_rays`
/// to pin the cone-matched LOD logic without spinning up a GPU.
pub fn pick_cascade_cpu(
    p_world: Vec3,
    cone_radius: f32,
    uniforms: &GdfUniforms,
) -> Option<u32> {
    for c in 0..CASCADE_COUNT {
        let cascade = uniforms.cascades[c];
        let cube_extent =
            cascade.voxel_count_per_axis as f32 * cascade.voxel_size;
        let origin = Vec3::from_array(cascade.world_origin);
        let aabb_max = origin + Vec3::splat(cube_extent);
        let inside = p_world.cmpge(origin).all() && p_world.cmple(aabb_max).all();
        if inside && cascade.voxel_size >= cone_radius {
            return Some(c as u32);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_stagger_runs_cascade_zero_only() {
        let mut sched = GdfScheduler::new();
        let dispatched = sched.cascades_to_update(Vec3::ZERO);
        // Frame 0 — only cascade 0 bootstraps. No other cascade has
        // its schedule, dirty, or drift trigger fire because they're
        // not bootstrapped yet.
        assert_eq!(dispatched.as_slice(), &[0]);
    }

    #[test]
    fn cascade_c_bootstraps_at_frame_c() {
        for target_c in 0..CASCADE_COUNT {
            let mut sched = GdfScheduler::new();
            for f in 0..target_c {
                let d = sched.cascades_to_update(Vec3::ZERO);
                assert!(
                    !d.contains(&(target_c as u32)),
                    "cascade {target_c} bootstrapped early at frame {f}: {d:?}"
                );
            }
            let d = sched.cascades_to_update(Vec3::ZERO);
            assert!(
                d.contains(&(target_c as u32)),
                "cascade {target_c} did not bootstrap at frame {target_c}: {d:?}"
            );
        }
    }

    #[test]
    fn round_robin_schedule_holds_60_frames() {
        let mut sched = GdfScheduler::new();
        let mut counts = [0u32; CASCADE_COUNT];
        for _ in 0..60 {
            let dispatched = sched.cascades_to_update(Vec3::ZERO);
            for c in dispatched {
                counts[c as usize] += 1;
            }
        }
        // 60 frames, cascade `c` bootstraps at frame `c`, then
        // re-runs every `2^c` frames thereafter.
        // Cascade 0: every frame → 60.
        // Cascade 1: bootstrap @ 1, schedule @ 2,4,...,58 → 1+29=30.
        // Cascade 2: bootstrap @ 2, schedule @ 4,8,...,56 → 1+14=15.
        // Cascade 3: bootstrap @ 3, schedule @ 8,16,24,32,40,48,56 → 1+7=8.
        // Cascade 4: bootstrap @ 4, schedule @ 16,32,48 → 1+3=4.
        // Cascade 5: bootstrap @ 5, schedule @ 32 → 2.
        assert_eq!(counts, [60, 30, 15, 8, 4, 2], "counts: {counts:?}");
    }

    #[test]
    fn dirty_chunk_forces_off_schedule_dispatch() {
        let mut sched = GdfScheduler::new();
        // Bootstrap every cascade so the dirty trigger isn't masked
        // by the bootstrap path.
        for _ in 0..32 {
            sched.cascades_to_update(Vec3::ZERO);
        }
        // Frame 32 dispatches every cascade via schedule (32 % 2^c
        // == 0 for c=0..5). Advance one more frame so cascades 1..5
        // are NOT due, then dirty + dispatch.
        sched.cascades_to_update(Vec3::ZERO); // frame 33
        sched.mark_chunk_dirty(0);
        let dispatched = sched.cascades_to_update(Vec3::ZERO); // frame 34
        // Cascade 0 always dispatches; cascade 1 is also due at 34
        // (34 % 2 == 0). Cascade 3 (% 8) is NOT due at 34 → should
        // dispatch via dirty trigger.
        assert!(
            dispatched.contains(&3),
            "dirty chunk did not force cascade 3 off-schedule dispatch: {dispatched:?}"
        );
    }

    #[test]
    fn camera_drift_forces_re_snap() {
        let mut sched = GdfScheduler::new();
        // Bootstrap all six cascades.
        for _ in 0..6 {
            sched.cascades_to_update(Vec3::ZERO);
        }
        // Frame 6 — cascade 1 was due at 6 (6 % 2 == 0). Step one
        // more frame so cascade 1's `last_centre` snapshot was just
        // taken at frame 6 with camera=ZERO.
        sched.cascades_to_update(Vec3::ZERO); // frame 7
        // Move camera 50 m. Cascade 1 drift threshold = 4 × 2 = 8 m.
        // 50 m > 8 m → re-snap forced even off-schedule (frame 7
        // is NOT a multiple of 2 after the +1 increment).
        let dispatched = sched.cascades_to_update(Vec3::new(50.0, 0.0, 0.0));
        // Frame 8 IS due for cascade 1 by schedule too — so to
        // pin the drift path specifically, advance one more frame.
        let dispatched_off = sched.cascades_to_update(Vec3::new(100.0, 0.0, 0.0));
        // Frame 9 — cascade 1 schedule is NOT due (9 % 2 != 0).
        // 100 m drift from prior 50 m position is 50 m > 8 m → re-snap.
        assert!(
            dispatched_off.contains(&1),
            "camera drift past cascade-1 threshold did not force off-schedule re-snap: \
             dispatched={dispatched:?} dispatched_off={dispatched_off:?}"
        );
    }
}
