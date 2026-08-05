//! Bind group layout entries for the populate + populate-finalize
//! compute pipelines. Split out so `populate.rs` stays focused on the
//! pass struct + record logic — the BGL boilerplate dominates length
//! without dominating substance.
//!
//! Binding numbers must mirror `sparse_populate.wgsl` (5..=9) and the
//! freelist helpers (`sparse_freelist.wgsl`, bindings 0 and 1). The
//! finalize layout mirrors classify's finalize — same shader, same
//! bindings, only the `FINALIZE_WORKGROUP_SIZE` override differs.

/// Bind group layout entries for the populate pass `@group(0)`.
pub(super) const POPULATE_BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 7] = [
    // sparse_free_list (read_write storage)
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // sparse_counters (read_write storage; atomics inside)
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // populate_root_indices (read_write — populate stores the alloced
    // subgrid_idx or the alloc-failed sentinel here).
    wgpu::BindGroupLayoutEntry {
        binding: 5,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // populate_subgrid_pool (write-only storage texture — atlas tile
    // destination, R16Float matches `SparseGrid::POOL_TEXTURE_FORMAT`).
    wgpu::BindGroupLayoutEntry {
        binding: 6,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R16Float,
            view_dimension: wgpu::TextureViewDimension::D3,
        },
        count: None,
    },
    // populate_needs_indices (read — written by classify)
    wgpu::BindGroupLayoutEntry {
        binding: 7,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // populate_needs_count (read — written by classify atomicAdd)
    wgpu::BindGroupLayoutEntry {
        binding: 8,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // populate uniform
    wgpu::BindGroupLayoutEntry {
        binding: 9,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
];

/// Bind group layout entries for the populate-finalize pass
/// `@group(0)`. Same WGSL as classify-finalize; only the
/// `FINALIZE_WORKGROUP_SIZE` pipeline override differs at compile.
pub(super) const FINALIZE_BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 2] = [
    // needs_count read
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // populate indirect args read_write
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
];
