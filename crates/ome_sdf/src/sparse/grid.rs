//! `SparseGrid` — chunk-local sparse SDF voxel storage with the
//! 4-LOD cascade introduced in S7 of issue #136.
//!
//! Owns the GPU buffers + 3D texture atlases backing the two-level
//! sparse layout, replicated per LOD. Mutating compute passes
//! (chunk_lod / classify / populate / downsample) live in sibling
//! modules and bind these resources; this module is the lifecycle
//! root that all of them compose against.
//!
//! # Per-LOD cascade
//!
//! Every `SparseGrid` carries [`LOD_COUNT`] (= 4) parallel sets of
//! resources — one per cascade level (see [`crate::sparse::lod`] for
//! the geometry table). Each LOD has its own:
//!
//! - `root_indices_buffer[lod]` — `ROOT_CELLS × u32`, the root-cell →
//!   subgrid_idx map at this LOD's atlas.
//! - `subgrid_pool_texture[lod]` — `R16Float` 3D atlas sized per
//!   [`LodConfig::atlas_dim_*`].
//! - `free_list_buffer[lod]`, `counters_buffer[lod]` — atomic
//!   freelist bookkeeping.
//! - `needs_indices_buffer[lod]`, `needs_count_buffer[lod]` —
//!   classify-pass compaction output, consumed by populate.
//! - `populate_indirect_args_buffer[lod]` —
//!   `[needs_count, 1, 1]` written by populate-finalize.
//!
//! Plus three resources shared across the cascade:
//!
//! - `subgrid_pool_sampler` — one `Linear + ClampToEdge` sampler.
//! - `chunk_lod_mask_buffer` — `u32` bitmask written by `ChunkLodPass`,
//!   bit `i` = "LOD `i` active for this chunk".
//! - `downsample_indirect_args_buffer[cascade]` — `[wg_count, 1, 1]`
//!   written by downsample-finalize for each cascade `(0→1, 1→2,
//!   2→3)`. Three buffers, indexed by source LOD.
//!
//! [`LOD_COUNT`]: crate::sparse::LOD_COUNT
//! [`LodConfig::atlas_dim_*`]: crate::sparse::LodConfig
//!
//! # Encoder ordering invariant
//!
//! Within a single submission, the canonical hot-loop order is
//! `chunk_lod → classify[0..3] → populate_finalize[0..3] →
//! populate[0..3] → downsample[0→1, 1→2, 2→3]` — 16 compute passes,
//! one queue submission, zero CPU readback. wgpu's implicit
//! storage-buffer + storage-texture barriers between consecutive
//! compute passes provide the required happens-before edges.

mod buffers;

use ome_bvh::Aabb;

use super::{LOD_COUNT, LOD_LEVELS, free_list};

/// Size in bytes of the dispatch-indirect-args triple `[x, y, z]`
/// (3 × `u32`).
pub const DISPATCH_INDIRECT_ARGS_SIZE: u64 = 12;

/// `r16float` is the canonical pool-atlas format. Mirrored here so
/// consumers (populate's storage-write binding, lookup's sampled
/// binding, downsample's textureLoad source + storage destination)
/// match without each carrying its own copy.
pub const POOL_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Number of downsample cascades (`LOD_COUNT - 1`). One per
/// adjacent-LOD pair — `(0→1, 1→2, 2→3)`.
pub const DOWNSAMPLE_CASCADES: usize = (LOD_COUNT as usize) - 1;

/// Size in bytes of the metrics buffer written by the metrics pass
/// (S8). `[active_per_lod[LOD_COUNT], alloc_count_total,
/// free_count_total]` — `(LOD_COUNT + 2) × u32`.
pub const METRICS_BUFFER_SIZE: u64 = ((LOD_COUNT as u64) + 2) * 4;

/// Fixed-capacity sparse SDF grid bound to one chunk. See module-level
/// docs for the layout and encoder-ordering contract.
pub struct SparseGrid {
    bounds: Aabb,
    max_subgrids: u32,
    root_indices_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    subgrid_pool_textures: [wgpu::Texture; LOD_COUNT as usize],
    subgrid_pool_views: [wgpu::TextureView; LOD_COUNT as usize],
    subgrid_pool_sampler: wgpu::Sampler,
    free_list_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    counters_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    needs_indices_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    needs_count_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    populate_indirect_args_buffers: [wgpu::Buffer; LOD_COUNT as usize],
    downsample_indirect_args_buffers: [wgpu::Buffer; DOWNSAMPLE_CASCADES],
    chunk_lod_mask_buffer: wgpu::Buffer,
    metrics_buffer: wgpu::Buffer,
}

impl SparseGrid {
    /// Allocate the per-LOD GPU resources for a fresh `SparseGrid`
    /// covering `bounds` (chunk-local f32, post-`ActiveOrigin`) and
    /// seed every LOD's freelist + counters so the cascade is
    /// immediately ready for a `chunk_lod → classify → populate →
    /// downsample` submission.
    ///
    /// `max_subgrids` is applied uniformly across all LODs (every LOD
    /// has the same `MAX_SUBGRIDS_PER_ATLAS = 1024` capacity by
    /// construction). Use [`crate::sparse::MAX_SUBGRIDS_DEFAULT`]
    /// unless profiling motivates a smaller per-chunk override; values
    /// above the atlas tile capacity panic.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: Aabb,
        max_subgrids: u32,
    ) -> Self {
        for (idx, lod) in LOD_LEVELS.iter().enumerate() {
            assert!(
                max_subgrids > 0 && max_subgrids <= lod.max_subgrids,
                "max_subgrids must be in 1..={}, got {max_subgrids}", lod.max_subgrids,
            );
            let _ = idx;
        }

        let root_indices_buffers = std::array::from_fn(|i| {
            buffers::make_root_indices_buffer(device, i as u32)
        });
        let pool_pairs: [_; LOD_COUNT as usize] = std::array::from_fn(|i| {
            buffers::make_subgrid_pool_texture(device, &LOD_LEVELS[i], i as u32)
        });
        let mut texs: [Option<wgpu::Texture>; LOD_COUNT as usize] =
            [const { None }; LOD_COUNT as usize];
        let mut views: [Option<wgpu::TextureView>; LOD_COUNT as usize] =
            [const { None }; LOD_COUNT as usize];
        for (i, (t, v)) in pool_pairs.into_iter().enumerate() {
            texs[i] = Some(t);
            views[i] = Some(v);
        }
        let subgrid_pool_textures = texs.map(|o| o.expect("texture initialised above"));
        let subgrid_pool_views = views.map(|o| o.expect("view initialised above"));
        let subgrid_pool_sampler = buffers::make_subgrid_pool_sampler(device);

        let free_list_buffers = std::array::from_fn(|i| {
            buffers::make_free_list_buffer(device, max_subgrids, i as u32)
        });
        let counters_buffers = std::array::from_fn(|i| {
            buffers::make_counters_buffer(device, i as u32)
        });
        let needs_indices_buffers = std::array::from_fn(|i| {
            buffers::make_needs_indices_buffer(device, i as u32)
        });
        let needs_count_buffers = std::array::from_fn(|i| {
            buffers::make_needs_count_buffer(device, i as u32)
        });
        let populate_indirect_args_buffers = std::array::from_fn(|i| {
            buffers::make_populate_indirect_args_buffer(device, i as u32)
        });
        let downsample_indirect_args_buffers = std::array::from_fn(|i| {
            buffers::make_downsample_indirect_args_buffer(device, i as u32)
        });
        let chunk_lod_mask_buffer = buffers::make_chunk_lod_mask_buffer(device);
        let metrics_buffer = buffers::make_metrics_buffer(device);

        let grid = Self {
            bounds,
            max_subgrids,
            root_indices_buffers,
            subgrid_pool_textures,
            subgrid_pool_views,
            subgrid_pool_sampler,
            free_list_buffers,
            counters_buffers,
            needs_indices_buffers,
            needs_count_buffers,
            populate_indirect_args_buffers,
            downsample_indirect_args_buffers,
            chunk_lod_mask_buffer,
            metrics_buffer,
        };

        for lod_idx in 0..LOD_COUNT {
            free_list::init(
                queue,
                &grid.free_list_buffers[lod_idx as usize],
                &grid.counters_buffers[lod_idx as usize],
                max_subgrids,
            );
        }
        grid
    }

    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// Per-LOD subgrid capacity (the constructor's `max_subgrids`
    /// argument, applied uniformly across LODs).
    pub fn max_subgrids(&self) -> u32 {
        self.max_subgrids
    }

    /// Per-LOD root → subgrid_idx map. `ROOT_CELLS × u32`. See
    /// module-level docs for the sentinel encoding.
    pub fn root_indices_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.root_indices_buffers[lod_idx as usize]
    }

    /// 3D atlas texture for `lod_idx` (`R16Float`). Sized per
    /// `LOD_LEVELS[lod_idx]`; bound as a storage texture by populate
    /// + downsample (write) and as a sampled texture by the lookup
    /// helper (read).
    pub fn subgrid_pool_texture(&self, lod_idx: u32) -> &wgpu::Texture {
        &self.subgrid_pool_textures[lod_idx as usize]
    }

    /// Default view over the LOD `lod_idx` atlas. Reusable for both
    /// `STORAGE_BINDING` and `TEXTURE_BINDING` since the texture
    /// declares both usage flags.
    pub fn subgrid_pool_view(&self, lod_idx: u32) -> &wgpu::TextureView {
        &self.subgrid_pool_views[lod_idx as usize]
    }

    /// One shared `Linear + ClampToEdge` sampler — every LOD's lookup
    /// binding reuses the same sampler instance (sampler state is
    /// LOD-independent).
    pub fn subgrid_pool_sampler(&self) -> &wgpu::Sampler {
        &self.subgrid_pool_sampler
    }

    pub fn free_list_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.free_list_buffers[lod_idx as usize]
    }

    pub fn counters_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.counters_buffers[lod_idx as usize]
    }

    /// `ROOT_CELLS × u32` compaction buffer per LOD. Filled by the
    /// classify pass at LOD `lod_idx` with the linear root-cell
    /// indices the surface intersects at this LOD's resolution; the
    /// populate pass at the same LOD consumes
    /// `[0..needs_count[lod_idx]]` of it via indirect dispatch.
    pub fn needs_indices_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.needs_indices_buffers[lod_idx as usize]
    }

    /// 4-byte atomic `u32` counter per LOD. Read by populate-finalize
    /// to derive the indirect dispatch args for the populate stage,
    /// and by downsample-finalize for cascade `lod_idx → lod_idx + 1`.
    pub fn needs_count_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.needs_count_buffers[lod_idx as usize]
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer per LOD,
    /// written by the populate-finalize compute pass. Bound with
    /// `BufferUsages::INDIRECT` so the populate pass can call
    /// `dispatch_workgroups_indirect(&buf, 0)` directly.
    pub fn populate_indirect_args_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.populate_indirect_args_buffers[lod_idx as usize]
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer per cascade
    /// `(0→1, 1→2, 2→3)`. Indexed by the *source* LOD: cascade 0 maps
    /// LOD 0 → LOD 1, cascade 1 maps LOD 1 → LOD 2, cascade 2 maps
    /// LOD 2 → LOD 3.
    pub fn downsample_indirect_args_buffer(
        &self,
        cascade_idx: u32,
    ) -> &wgpu::Buffer {
        &self.downsample_indirect_args_buffers[cascade_idx as usize]
    }

    /// Per-chunk LOD bitmask, written by [`ChunkLodPass`]. Bit `i`
    /// (LSB-first) means "LOD `i` is active for this chunk". Bit 0 is
    /// always set — the cascade's downsample stages assume LOD 0 is
    /// populated as the cascade source.
    ///
    /// [`ChunkLodPass`]: crate::sparse::ChunkLodPass
    pub fn chunk_lod_mask_buffer(&self) -> &wgpu::Buffer {
        &self.chunk_lod_mask_buffer
    }

    /// 24-byte metrics buffer. Layout matches the WGSL `SparseMetrics`
    /// struct in `sparse_metrics.wgsl`:
    /// `[active_lod0..3, alloc_count_total, free_count_total]`. Written
    /// by [`MetricsPass::record`] at the tail of the cascade and read
    /// asynchronously by [`Metrics::read`].
    ///
    /// [`MetricsPass::record`]: crate::sparse::MetricsPass::record
    /// [`Metrics::read`]: crate::sparse::Metrics::read
    pub fn metrics_buffer(&self) -> &wgpu::Buffer {
        &self.metrics_buffer
    }
}

#[cfg(test)]
mod tests;
