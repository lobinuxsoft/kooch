//! `SparseGrid` — chunk-local sparse SDF voxel storage.
//!
//! Owns the GPU buffers + 3D texture atlas backing the two-level
//! sparse layout. Mutating compute passes (classify / allocate /
//! populate / free) live in sibling modules and bind these resources;
//! this module is the lifecycle root that all of them compose
//! against.

use ome_bvh::Aabb;

use super::{
    ATLAS_DIM_X, ATLAS_DIM_Y, ATLAS_DIM_Z, MAX_SUBGRIDS_PER_ATLAS, ROOT_CELLS, free_list,
};

/// Size in bytes of the dispatch-indirect-args triple
/// `[x, y, z]` (3 × `u32`). Constant rather than `mem::size_of`-derived
/// so consumers can match the layout without depending on a host
/// helper type.
pub const DISPATCH_INDIRECT_ARGS_SIZE: u64 = 12;

/// `r16float` is the canonical pool-atlas format. Mirrored here so
/// consumers (populate's storage-write binding, lookup's sampled
/// binding) match without each carrying its own copy.
pub const POOL_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Fixed-capacity sparse SDF grid bound to one chunk. See module-level
/// docs in [`super`] for the layout, capacity, and encoder-ordering
/// contract.
pub struct SparseGrid {
    bounds: Aabb,
    max_subgrids: u32,
    root_indices_buffer: wgpu::Buffer,
    subgrid_pool_texture: wgpu::Texture,
    subgrid_pool_view: wgpu::TextureView,
    subgrid_pool_sampler: wgpu::Sampler,
    free_list_buffer: wgpu::Buffer,
    counters_buffer: wgpu::Buffer,
    needs_indices_buffer: wgpu::Buffer,
    needs_count_buffer: wgpu::Buffer,
    needs_indirect_args_buffer: wgpu::Buffer,
    populate_indirect_args_buffer: wgpu::Buffer,
}

impl SparseGrid {
    /// Allocate the four GPU buffers for a fresh `SparseGrid` covering
    /// `bounds` (chunk-local f32, post-`ActiveOrigin`) and seed the
    /// free list + counters so the grid is immediately ready for an
    /// allocate / populate cycle.
    ///
    /// `root_indices` is initialised to `EMPTY_ROOT_SENTINEL`
    /// (`0xFFFFFFFF`) via `mapped_at_creation` — every lookup on a
    /// fresh grid returns `FAR_FROM_SURFACE`. The free list is filled
    /// with the identity permutation `[0, 1, …, max_subgrids - 1]`
    /// and `counters.free_top` is set to `max_subgrids` via
    /// [`free_list::init`].
    ///
    /// `max_subgrids` must be in `1..=MAX_SUBGRIDS_PER_ATLAS` (1024).
    /// Use [`super::MAX_SUBGRIDS_DEFAULT`] unless profiling motivates
    /// a smaller per-chunk override; values above the atlas tile
    /// capacity panic at construction.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: Aabb,
        max_subgrids: u32,
    ) -> Self {
        assert!(
            max_subgrids > 0 && max_subgrids <= MAX_SUBGRIDS_PER_ATLAS,
            "max_subgrids must be in 1..={MAX_SUBGRIDS_PER_ATLAS}, got {max_subgrids}",
        );
        let (subgrid_pool_texture, subgrid_pool_view) = make_subgrid_pool_texture(device);
        let subgrid_pool_sampler = make_subgrid_pool_sampler(device);
        let grid = Self {
            bounds,
            max_subgrids,
            root_indices_buffer: make_root_indices_buffer(device),
            subgrid_pool_texture,
            subgrid_pool_view,
            subgrid_pool_sampler,
            free_list_buffer: make_free_list_buffer(device, max_subgrids),
            counters_buffer: make_counters_buffer(device),
            needs_indices_buffer: make_needs_indices_buffer(device),
            needs_count_buffer: make_needs_count_buffer(device),
            needs_indirect_args_buffer: make_needs_indirect_args_buffer(device),
            populate_indirect_args_buffer: make_populate_indirect_args_buffer(device),
        };
        free_list::init(queue, &grid.free_list_buffer, &grid.counters_buffer, max_subgrids);
        grid
    }

    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    pub fn max_subgrids(&self) -> u32 {
        self.max_subgrids
    }

    pub fn root_indices_buffer(&self) -> &wgpu::Buffer {
        &self.root_indices_buffer
    }

    /// 3D atlas texture (`R16Float`, `544 × 17 × 544`) holding all
    /// allocated subgrid voxels packed into 17³ tiles. Bound as a
    /// storage texture by the populate pass and as a sampled texture
    /// by the lookup helper.
    pub fn subgrid_pool_texture(&self) -> &wgpu::Texture {
        &self.subgrid_pool_texture
    }

    /// Default view over the full pool atlas. Reusable for both
    /// `STORAGE_BINDING` and `TEXTURE_BINDING` since the texture
    /// declares both usage flags.
    pub fn subgrid_pool_view(&self) -> &wgpu::TextureView {
        &self.subgrid_pool_view
    }

    /// `Linear` + `ClampToEdge` sampler matching the lookup helper's
    /// expectations. `MagFilter::Linear` does the trilinear blend in
    /// hardware; `ClampToEdge` keeps boundary samples inside the
    /// current tile's voxels (the lookup body additionally clamps the
    /// fractional offset to `[0, SUBGRID_DIM]` to keep tex coords off
    /// the next-tile texel boundary).
    pub fn subgrid_pool_sampler(&self) -> &wgpu::Sampler {
        &self.subgrid_pool_sampler
    }

    pub fn free_list_buffer(&self) -> &wgpu::Buffer {
        &self.free_list_buffer
    }

    pub fn counters_buffer(&self) -> &wgpu::Buffer {
        &self.counters_buffer
    }

    /// `ROOT_CELLS × u32` compaction buffer. Filled by the classify
    /// pass with the linear root-cell indices that need a subgrid
    /// allocation; the allocate pass (S4) consumes
    /// `[0..needs_count]` of it via indirect dispatch.
    pub fn needs_indices_buffer(&self) -> &wgpu::Buffer {
        &self.needs_indices_buffer
    }

    /// 4-byte atomic `u32` counter incremented once per marked cell by
    /// the classify pass. Read by the finalize compute pass to derive
    /// the indirect dispatch args.
    pub fn needs_count_buffer(&self) -> &wgpu::Buffer {
        &self.needs_count_buffer
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer written by the
    /// classify-finalize compute pass. Bound with
    /// `BufferUsages::INDIRECT` so the allocate pass can call
    /// `dispatch_workgroups_indirect(&buf, 0)` directly.
    pub fn needs_indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.needs_indirect_args_buffer
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer written by the
    /// populate-finalize compute pass. Distinct from
    /// `needs_indirect_args_buffer` because the two consumers use
    /// different workgroup sizes — classify's downstream consumer is
    /// the allocate pass at `@workgroup_size(64)` (so x = ⌈n / 64⌉),
    /// populate is `1 workgroup per marked cell` (x = n). Bound with
    /// `BufferUsages::INDIRECT` for `dispatch_workgroups_indirect`.
    pub fn populate_indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.populate_indirect_args_buffer
    }
}

fn make_root_indices_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let size = (ROOT_CELLS as u64) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::root_indices"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: true,
    });
    {
        // 0xFFFFFFFF is byte-pattern 0xFF, so a flat byte fill is the
        // correct initialiser for every u32 entry. `BufferViewMut` is
        // write-only in wgpu 29 (mapped memory may be write-combining
        // and does not support `&mut [u8]`), so we copy from a small
        // staging vector — `ROOT_CELLS × 4 = 16 KiB`, trivially cheap.
        let init = vec![0xFFu8; size as usize];
        buffer.slice(..).get_mapped_range_mut().copy_from_slice(&init);
    }
    buffer.unmap();
    buffer
}

fn make_subgrid_pool_texture(
    device: &wgpu::Device,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ome_sdf::sparse::subgrid_pool"),
        size: wgpu::Extent3d {
            width: ATLAS_DIM_X,
            height: ATLAS_DIM_Y,
            depth_or_array_layers: ATLAS_DIM_Z,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: POOL_TEXTURE_FORMAT,
        // STORAGE for populate's `textureStore` writes; TEXTURE for
        // the lookup's sampled reads via `textureSampleLevel`.
        // COPY_SRC/COPY_DST keep test readback + future reset paths
        // available without a second texture allocation.
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ome_sdf::sparse::subgrid_pool_view"),
        format: Some(POOL_TEXTURE_FORMAT),
        dimension: Some(wgpu::TextureViewDimension::D3),
        usage: None,
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: None,
    });
    (texture, view)
}

fn make_subgrid_pool_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ome_sdf::sparse::subgrid_pool_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 0.0,
        compare: None,
        anisotropy_clamp: 1,
        border_color: None,
    })
}

fn make_free_list_buffer(device: &wgpu::Device, max_subgrids: u32) -> wgpu::Buffer {
    let size = (max_subgrids as u64) * 4;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::free_list"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_counters_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::counters"),
        // 4 × u32: free_top, alloc_failed_count, _pad, _pad.
        size: 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_indices_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    // Worst case: every root cell needs allocation → ROOT_CELLS × 4 B
    // = 16 KiB. Fixed-capacity sized to that bound; a smaller capacity
    // would only complicate the classify shader's slot bookkeeping
    // without saving meaningful memory.
    let size = (ROOT_CELLS as u64) * 4;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_indices"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_count_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_count"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_indirect_args_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_indirect_args"),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_populate_indirect_args_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::populate_indirect_args"),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests;
