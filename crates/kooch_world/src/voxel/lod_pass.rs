//! 7-pass cascade orchestrator.
//!
//! Owns the four constituent passes — [`ChunkLodPass`],
//! [`ClassifyPass`], [`PopulatePass`], [`DownsamplePass`] — and
//! exposes a single [`record`] entry point that records the canonical
//! cascade order into one command encoder:
//!
//! ```text
//! 1. chunk_lod
//! 2. classify_lod0
//! 3. populate_finalize_lod0
//! 4. populate_lod0
//! 5. downsample 0→1
//! 6. downsample 1→2
//! 7. downsample 2→3
//! 8. metrics
//! ```
//!
//! 8 compute passes, one queue submission, zero CPU readback in the
//! hot loop. The metrics pass writes into `metrics_buffer` only —
//! readback is opt-in async via [`Metrics::read`], never per-frame.
//!
//! # Invariant — `base_lod = 0`
//!
//! Single-chunk today: every cascade always classifies + populates at
//! LOD 0 only, then box-filters into LODs 1..3 via the downsample
//! chain. Running classify / populate at LOD > 0 is redundant — the
//! cell set is identical (root grid is LOD-independent) and the
//! downsample's writes overwrite anything populate would have produced
//! at higher LODs anyway. Skipping LOD > 0 producers also kills the
//! free-list leak that previously stranded slots in
//! `free_list_lod_{1,2,3}` whenever those LODs were active in the
//! mask alongside LOD 0.
//!
//! Per-LOD classify and populate pipelines stay instantiated in
//! [`Self::new`] — direct tests of `ClassifyPass` / `PopulatePass`
//! exercise them, and the per-chunk `base_lod_idx` selector landing
//! with #313 (multi-chunk) will need them for distant chunks where
//! the cascade source moves above LOD 0.
//!
//! # When to call
//!
//! Once per chunk per bake. The Edit Baker (#309) calls `record`
//! after writing the per-frame `active_origin` for the active player,
//! consuming the populated atlas via [`super::lookup_wgsl`]
//! immediately afterwards (same submission, encoder ordering covered
//! by wgpu's compute-pass barrier).
//!
//! [`ChunkLodPass`]: super::ChunkLodPass
//! [`ClassifyPass`]: super::ClassifyPass
//! [`PopulatePass`]: super::PopulatePass
//! [`DownsamplePass`]: super::DownsamplePass

use glam::Vec3;

use super::{
    CASCADE_COUNT, ChunkLodPass, ClassifyPass, DEFAULT_LOD_DISTANCE_THRESHOLDS, DEFAULT_MARGIN,
    DownsamplePass, MetricsPass, PopulatePass, SparseGrid,
};

/// Compose all five cascade passes into one orchestrator. One
/// instance per device — bind groups are rebuilt per-record call so
/// the orchestrator is grid-agnostic.
pub struct SparseLodPass {
    chunk_lod: ChunkLodPass,
    classify: ClassifyPass,
    populate: PopulatePass,
    downsample: DownsamplePass,
    metrics: MetricsPass,
}

impl SparseLodPass {
    /// Build the orchestrator. `sampler_wgsl` and
    /// `sampler_bgl_entries` are forwarded to both [`ClassifyPass::new`]
    /// and [`PopulatePass::new`] — they share the same sampler
    /// `@group(1)` layout.
    pub fn new(
        device: &wgpu::Device,
        sampler_wgsl: &str,
        sampler_bgl_entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Self {
        let chunk_lod = ChunkLodPass::new(device);
        let classify = ClassifyPass::new(device, sampler_wgsl, sampler_bgl_entries);
        let populate = PopulatePass::new(device, sampler_wgsl, sampler_bgl_entries);
        let downsample = DownsamplePass::new(device);
        let metrics = MetricsPass::new(device);
        Self {
            chunk_lod,
            classify,
            populate,
            downsample,
            metrics,
        }
    }

    /// Record the full 7-pass cascade for `grid` into `encoder`.
    /// Caller submits the encoder + handles synchronisation with
    /// downstream lookup pipelines.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        active_origin: Vec3,
        thresholds: [f32; 3],
        margin: f32,
    ) {
        // Pass 1: chunk_lod — write the per-chunk LOD bitmask.
        self.chunk_lod
            .record(device, queue, encoder, grid, active_origin, thresholds);

        // Pass 2: classify at LOD 0 — every chunk has bit 0 set in
        // the mask (cascade invariant), so this is the only producer
        // run. LODs 1..3 inherit the marked cell set via the
        // downsample chain.
        self.classify
            .record(device, queue, encoder, grid, sampler_bg, 0, margin);

        // Pass 3: populate-finalize at LOD 0 — derive
        // `[needs_count_lod0, 1, 1]` into populate_indirect_args[0].
        self.populate.record_finalize(device, encoder, grid, 0);

        // Pass 4: populate at LOD 0 — fill atlas[0] via indirect
        // dispatch.
        self.populate
            .record_populate(device, queue, encoder, grid, sampler_bg, 0);

        // Passes 5..=7: downsample[0→1, 1→2, 2→3] — box-filter
        // cascade fills LODs 1..3 from LOD 0's populated tiles. Each
        // cascade reuses populate_indirect_args[lod_src] (already
        // [needs_count_src, 1, 1]) — no extra finalize needed.
        for cascade_idx in 0..(CASCADE_COUNT as u32) {
            self.downsample
                .record_cascade(device, encoder, grid, cascade_idx);
        }

        // Pass 8: metrics — telemetry sink. Reads each LOD's freelist
        // counters + cumulative alloc/free totals and writes the 24 B
        // `SparseMetrics` struct. Lookup hot path never reads it; host
        // pulls via `Metrics::read` at telemetry cadence (off-thread).
        self.metrics.record(device, encoder, grid);
    }

    /// Convenience overload — runs [`record`] with the
    /// `DEFAULT_LOD_DISTANCE_THRESHOLDS` and [`DEFAULT_MARGIN`]. Most
    /// production call sites use the defaults; tuning happens at
    /// telemetry time, not per-frame.
    pub fn record_with_defaults(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        active_origin: Vec3,
    ) {
        self.record(
            device,
            queue,
            encoder,
            grid,
            sampler_bg,
            active_origin,
            DEFAULT_LOD_DISTANCE_THRESHOLDS,
            DEFAULT_MARGIN,
        );
    }

    /// Sampler bind group layout shared by classify + populate. The
    /// caller must build their sampler bind group against this layout
    /// (or one structurally equal to it).
    pub fn sampler_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.classify.sampler_bind_group_layout()
    }

    pub fn chunk_lod_pass(&self) -> &ChunkLodPass {
        &self.chunk_lod
    }
    pub fn classify_pass(&self) -> &ClassifyPass {
        &self.classify
    }
    pub fn populate_pass(&self) -> &PopulatePass {
        &self.populate
    }
    pub fn downsample_pass(&self) -> &DownsamplePass {
        &self.downsample
    }
    pub fn metrics_pass(&self) -> &MetricsPass {
        &self.metrics
    }
}
