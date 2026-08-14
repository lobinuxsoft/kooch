use super::super::stage_counters::CullStageCounts;

/// Per-frame return value reporting how the stage spent its budget.
/// Surfaced through the editor's debug-stats overlay (#451) and used
/// by the integration test as a render side-effect.
#[derive(Copy, Clone, Debug, Default)]
pub struct MeshletRenderStats {
    /// Number of `MeshInstance` records uploaded this frame.
    pub instances_uploaded: u32,
    /// Worst-case `(instance_count × meshlets_per_mesh)` thread budget
    /// the cull dispatch saw — equals the upper bound on surviving
    /// meshlets before any cull rejection.
    pub cull_threads: u32,
    /// World-space camera position the cull / LOD selector saw this
    /// frame. Surfaced in the View toolbar so the artist can confirm
    /// the selector is actually following the active camera (the LOD
    /// boundary rule is distance-driven; if this stays static while
    /// the editor camera moves, the selector is reading the wrong
    /// view matrix).
    pub cam_pos: [f32; 3],
    /// Total meshlet count across the entire `GlobalMeshPool` (every
    /// LOD of every registered mesh, concatenated).
    pub pool_meshlets_total: u32,
    /// Subset of `pool_meshlets_total` whose `parent_meshlet_index`
    /// is the sentinel — terminal stops for the runtime selector.
    /// `roots == total` ⇒ the chain has no usable depth (every mesh
    /// is single-LOD or every group failed to simplify). `roots <<
    /// total` ⇒ the chain has depth and the selector should be able
    /// to descend / ascend across distance.
    pub pool_meshlets_roots: u32,
    /// Wall-clock duration of the cull → vbuf raster → deferred
    /// shade chain on the GPU, in milliseconds. `None` when GPU
    /// timers are disabled (no `Features::TIMESTAMP_QUERY` support
    /// or `enable_gpu_timers` was never called) or the first ring
    /// readback hasn't landed yet (1-2 frames after enable).
    pub gpu_frame_ms: Option<f32>,
    /// Number of dispatch / render-pass operations emitted by the
    /// meshlet pipeline this frame (#463.6). Indirect dispatch
    /// means this is bounded — one cull, one vbuf raster, one
    /// deferred shade — regardless of instance count. Sky / gizmo /
    /// blit / egui passes outside the meshlet stage are counted
    /// separately by the editor render system.
    pub draw_calls: u32,
    /// Lights in the busiest froxel, and the mean over the froxels that
    /// hold any (#820).
    ///
    /// The number the lights-per-pixel view (#817) could only be
    /// bisected for by eye, which does not separate 32 from 45 — and
    /// which cannot be compared before and after a change to the grid
    /// without doing the bisection twice.
    ///
    /// `None` when the frame did not cluster (no camera matrices, or
    /// clustering off) or the async readback has not landed yet, 1-2
    /// frames in. Stale by the same design as the cull counts: the
    /// alternative is a `device.poll` in the hot loop.
    pub cluster_occupancy: Option<(u32, f32)>,
    /// Per-stage cull survivor counts (#454.6).
    /// `[after_frustum, after_backface, after_hi_z, total_visible]`.
    /// `None` when no debug-active mode has been selected yet (the
    /// cull never wrote the buffer) or the first async readback
    /// hasn't landed (1-2 frames after the user picks a reject
    /// mode). The values are 1-2 frames stale by design — the
    /// readback ring trades freshness for skipping `device.poll`
    /// stalls in the hot loop.
    pub cull_stage_counts: Option<CullStageCounts>,
    /// Per-pass GPU timings in milliseconds (#252). Each entry is
    /// `(label, ms)` for one stage of the meshlet frame; the labels
    /// describe the path-specific breakdown:
    ///
    /// - R64 atomic path: `[("Cull", _), ("Raster", _), ("Overlay", _)]`.
    /// - Hi-Z 2-pass path: `[("Pass A", _), ("Hi-Z", _), ("Pass B", _)]`.
    ///
    /// Sum equals `gpu_frame_ms`; surfaced separately so the HUD can
    /// render each pass on its own row without having to know which
    /// path the frame took. Fixed-size `[_; 3]` keeps the stats
    /// struct `Copy` — both paths use exactly 3 stages today; if
    /// either grows past 3 the array widens here and the writer
    /// path zero-pads unused slots. `None` until the first ring
    /// readback completes (1-2 frames after `enable_gpu_timers`) or
    /// when GPU timers are disabled.
    pub stage_timings: Option<[(&'static str, f32); 3]>,
}
