//! Per-LOD GPU buffer + texture allocation helpers for `SparseGrid`.
//! Split out so `grid.rs` keeps the struct + accessor surface small —
//! the buffer-creation boilerplate dominates length without
//! contributing to the public API.

use crate::voxel::{LodConfig, ROOT_CELLS};

use super::{DISPATCH_INDIRECT_ARGS_SIZE, POOL_TEXTURE_FORMAT};

/// Build the per-LOD root-indices buffer, pre-initialised to
/// `EMPTY_ROOT_SENTINEL` (`0xFFFFFFFF`). Same layout for every LOD —
/// `ROOT_CELLS × u32` — so a fresh grid reads as fully empty across
/// the cascade until classify + populate run.
pub(super) fn make_root_indices_buffer(device: &wgpu::Device, lod_idx: u32) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::root_indices_lod{lod_idx}");
    let size = (ROOT_CELLS as u64) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: true,
    });
    {
        // 0xFFFFFFFF is byte-pattern 0xFF, so a flat byte fill is the
        // correct initialiser for every u32. Mapped memory is
        // write-combining in wgpu 29, so we copy from a small staging
        // vector — `ROOT_CELLS × 4 = 16 KiB`, trivially cheap.
        let init = vec![0xFFu8; size as usize];
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(&init);
    }
    buffer.unmap();
    buffer
}

/// Build the per-LOD subgrid pool atlas (3D `R16Float` texture). The
/// `STORAGE_BINDING` usage covers the populate-pass `textureStore`
/// writes; `TEXTURE_BINDING` covers lookup + downsample reads.
pub(super) fn make_subgrid_pool_texture(
    device: &wgpu::Device,
    lod: &LodConfig,
    lod_idx: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let label = format!("kooch_world::voxel::subgrid_pool_lod{lod_idx}");
    let view_label = format!("{label}_view");
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label),
        size: wgpu::Extent3d {
            width: lod.atlas_dim_x,
            height: lod.atlas_dim_y,
            depth_or_array_layers: lod.atlas_dim_z,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: POOL_TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some(&view_label),
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

/// One shared `Linear + ClampToEdge` sampler used by every LOD's
/// lookup binding. Sampler state is filter mode + address mode only —
/// no per-LOD parameters — so a single instance suffices.
pub(super) fn make_subgrid_pool_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("kooch_world::voxel::subgrid_pool_sampler"),
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

pub(super) fn make_free_list_buffer(
    device: &wgpu::Device,
    max_subgrids: u32,
    lod_idx: u32,
) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::free_list_lod{lod_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size: (max_subgrids as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_counters_buffer(device: &wgpu::Device, lod_idx: u32) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::counters_lod{lod_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        // 4 × u32: free_top, alloc_failed_count, _pad, _pad.
        size: 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_needs_indices_buffer(device: &wgpu::Device, lod_idx: u32) -> wgpu::Buffer {
    // Worst case per LOD: every root cell needs allocation →
    // ROOT_CELLS × 4 B = 16 KiB. Same shape across LODs since the root
    // grid resolution does not depend on LOD.
    let label = format!("kooch_world::voxel::needs_indices_lod{lod_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size: (ROOT_CELLS as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_needs_count_buffer(device: &wgpu::Device, lod_idx: u32) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::needs_count_lod{lod_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_populate_indirect_args_buffer(
    device: &wgpu::Device,
    lod_idx: u32,
) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::populate_indirect_args_lod{lod_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_downsample_indirect_args_buffer(
    device: &wgpu::Device,
    cascade_idx: u32,
) -> wgpu::Buffer {
    let label = format!("kooch_world::voxel::downsample_indirect_args_c{cascade_idx}");
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&label),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn make_chunk_lod_mask_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kooch_world::voxel::chunk_lod_mask"),
        // 1 × u32 today (single chunk). Sized as `array<u32>` once
        // multi-chunk lands (#313) — same buffer, larger size.
        size: 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// 24-byte metrics buffer written by the metrics pass (S8). Layout:
/// `[active_lod0, active_lod1, active_lod2, active_lod3,
/// alloc_count_total, free_count_total]` (6 × u32). `STORAGE` for the
/// shader write + `COPY_SRC` so the host can copy into a MAP_READ
/// staging buffer (WebGPU forbids MAP_READ + STORAGE on the same
/// buffer). Telemetry only — never read by the lookup hot path.
pub(super) fn make_metrics_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kooch_world::voxel::metrics"),
        size: super::METRICS_BUFFER_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
