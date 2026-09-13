//! `SparseGrid`: per LOD, a root-cell → subgrid map, an `R16Float` atlas and freelist buffers. 8
//! passes, one submission, no readback: chunk_lod, classify, finalize, populate (LOD 0), downsample
//! ×3, metrics.

mod buffers;

use kooch_core::Aabb;

use super::{LOD_COUNT, LOD_LEVELS, free_list};

/// Size in bytes of the dispatch-indirect-args triple `[x, y, z]`
/// (3 × `u32`).
pub const DISPATCH_INDIRECT_ARGS_SIZE: u64 = 12;

/// The atlas format, shared so populate, lookup and downsample cannot bind different copies.
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
    /// Allocates every LOD's resources for `bounds` and seeds the freelists, ready for a cascade.
    /// `max_subgrids` applies to all LODs — use [`crate::voxel::MAX_SUBGRIDS_DEFAULT`]; above the
    /// tile capacity it panics.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: Aabb,
        max_subgrids: u32,
    ) -> Self {
        for (idx, lod) in LOD_LEVELS.iter().enumerate() {
            assert!(
                max_subgrids > 0 && max_subgrids <= lod.max_subgrids,
                "max_subgrids must be in 1..={}, got {max_subgrids}",
                lod.max_subgrids,
            );
            let _ = idx;
        }

        let root_indices_buffers =
            std::array::from_fn(|i| buffers::make_root_indices_buffer(device, i as u32));
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

        let free_list_buffers =
            std::array::from_fn(|i| buffers::make_free_list_buffer(device, max_subgrids, i as u32));
        let counters_buffers =
            std::array::from_fn(|i| buffers::make_counters_buffer(device, i as u32));
        let needs_indices_buffers =
            std::array::from_fn(|i| buffers::make_needs_indices_buffer(device, i as u32));
        let needs_count_buffers =
            std::array::from_fn(|i| buffers::make_needs_count_buffer(device, i as u32));
        let populate_indirect_args_buffers =
            std::array::from_fn(|i| buffers::make_populate_indirect_args_buffer(device, i as u32));
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

    /// `R16Float` atlas for `lod_idx`: a storage texture to populate and downsample, sampled by the
    /// lookup.
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

    /// Linear root-cell indices classify found on the surface, consumed by populate as
    /// `[0..needs_count[lod_idx]]`.
    pub fn needs_indices_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.needs_indices_buffers[lod_idx as usize]
    }

    /// 4-byte atomic `u32` counter per LOD. Read by populate-finalize
    /// to derive the indirect dispatch args for the populate stage,
    /// and by downsample-finalize for cascade `lod_idx → lod_idx + 1`.
    pub fn needs_count_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.needs_count_buffers[lod_idx as usize]
    }

    /// `[x, y, z]` args from populate-finalize, `INDIRECT` so populate dispatches from it directly.
    pub fn populate_indirect_args_buffer(&self, lod_idx: u32) -> &wgpu::Buffer {
        &self.populate_indirect_args_buffers[lod_idx as usize]
    }

    /// `[x, y, z]` args per cascade, indexed by source LOD.
    pub fn downsample_indirect_args_buffer(&self, cascade_idx: u32) -> &wgpu::Buffer {
        &self.downsample_indirect_args_buffers[cascade_idx as usize]
    }

    /// LOD bitmask from [`ChunkLodPass`](crate::voxel::ChunkLodPass); bit 0 is always set, since
    /// downsample sources from LOD 0.
    pub fn chunk_lod_mask_buffer(&self) -> &wgpu::Buffer {
        &self.chunk_lod_mask_buffer
    }

    /// 24 B `SparseMetrics` — active per LOD, then alloc and free totals — written by
    /// [`MetricsPass::record`](crate::voxel::MetricsPass::record) and read by
    /// [`Metrics::read`](crate::voxel::Metrics::read).
    pub fn metrics_buffer(&self) -> &wgpu::Buffer {
        &self.metrics_buffer
    }
}

#[cfg(test)]
mod tests;
