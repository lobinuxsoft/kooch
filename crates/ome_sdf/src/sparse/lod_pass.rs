//! 16-pass cascade orchestrator.
//!
//! Owns the four constituent passes — [`ChunkLodPass`],
//! [`ClassifyPass`], [`PopulatePass`], [`DownsamplePass`] — and
//! exposes a single [`record`] entry point that records the canonical
//! cascade order into one command encoder:
//!
//! ```text
//!  1.  chunk_lod
//!  2-5  classify[0..3]
//!  6-9  populate_finalize[0..3]
//! 10-13 populate[0..3]
//! 14-16 downsample[0→1, 1→2, 2→3]
//! ```
//!
//! 16 compute passes, one queue submission, zero CPU readback in the
//! hot loop. The split between `populate_finalize[0..3]` and
//! `populate[0..3]` lets the GPU pipeline finalize-derived indirect
//! arg writes ahead of the populate dispatches that consume them — wgpu
//! inserts the implicit storage-buffer barrier between consecutive
//! compute passes, so all four populate dispatches see fresh args
//! without the host pinning a fence in between.
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
    CASCADE_COUNT, ChunkLodPass, ClassifyPass, DEFAULT_LOD_DISTANCE_THRESHOLDS,
    DEFAULT_MARGIN, DownsamplePass, LOD_COUNT, PopulatePass, SparseGrid,
};

/// Compose all four cascade passes into one orchestrator. One
/// instance per device — bind groups are rebuilt per-record call so
/// the orchestrator is grid-agnostic.
pub struct SparseLodPass {
    chunk_lod: ChunkLodPass,
    classify: ClassifyPass,
    populate: PopulatePass,
    downsample: DownsamplePass,
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
        Self {
            chunk_lod,
            classify,
            populate,
            downsample,
        }
    }

    /// Record the full 16-pass cascade for `grid` into `encoder`.
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
        self.chunk_lod.record(device, queue, encoder, grid, active_origin, thresholds);

        // Passes 2..=5: classify[0..3] — mark cells per LOD, gated by
        // the chunk_lod_mask bit for that LOD.
        for lod_idx in 0..LOD_COUNT {
            self.classify.record(
                device, queue, encoder, grid, sampler_bg, lod_idx, margin,
            );
        }

        // Passes 6..=9: populate_finalize[0..3] — derive per-LOD
        // indirect args from each LOD's needs_count.
        for lod_idx in 0..LOD_COUNT {
            self.populate.record_finalize(device, encoder, grid, lod_idx);
        }

        // Passes 10..=13: populate[0..3] — fill each LOD's atlas via
        // its indirect dispatch.
        for lod_idx in 0..LOD_COUNT {
            self.populate.record_populate(
                device, queue, encoder, grid, sampler_bg, lod_idx,
            );
        }

        // Passes 14..=16: downsample[0→1, 1→2, 2→3] — box-filter
        // cascade fills the higher LODs from LOD 0's populated tiles.
        for cascade_idx in 0..(CASCADE_COUNT as u32) {
            self.downsample.record_cascade(device, encoder, grid, cascade_idx);
        }
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
            device, queue, encoder, grid, sampler_bg,
            active_origin, DEFAULT_LOD_DISTANCE_THRESHOLDS, DEFAULT_MARGIN,
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
}
