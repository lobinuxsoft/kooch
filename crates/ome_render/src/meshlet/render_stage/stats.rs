/// Per-frame return value reporting how the stage spent its budget.
/// Surfaced through the editor's debug-stats overlay (#451) and used
/// by the integration test as a render side-effect.
///
/// Per-stage cull survivor counts (frustum / backface / hi-z) require
/// a 4-byte CPU readback per frame and ship in #451b together with
/// the reject-reason tagging buffer.
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
}
