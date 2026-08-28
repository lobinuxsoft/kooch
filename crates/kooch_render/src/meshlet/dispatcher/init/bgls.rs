use std::num::NonZeroU64;

use crate::meshlet::cull::CullParams;
use crate::meshlet::scene::SceneCullParams;

use super::super::types::HiZTestParams;

/// Two-binding BGL for the cull-side debug buffers (#454.4 + #454.6).
///
/// - `binding(0)` = `reject_reasons` (one u32 per cull thread,
///   #454.4). Recorded on every return path when
///   `params.debug_active != 0`; consumed by the reject-overlay
///   raster pass.
/// - `binding(1)` = `stage_counters` (atomic `[u32; 4]`, #454.6).
///   AtomicAdded at each cull-stage tail (after-frustum,
///   after-backface, after-hi_z, total-visible) when
///   `params.debug_active != 0`; readback to CPU drives the editor's
///   per-stage survivor stats overlay.
///
/// Both are storage + read_write because the cull entry writes
/// through them. The reject-overlay pipeline reuses this same BGL
/// (declares both globals; only reads `reject_reasons`) so we don't
/// duplicate layouts.
pub(super) fn build_debug_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage_rw = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: NonZeroU64::new(4),
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_debug_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: storage_rw,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            },
        ],
    })
}

/// Single-binding BGL for the per-group `group_max_err` buffer.
/// Storage + read_write because pass 1 atomicMaxes into it; pass 2
/// also binds it as read_write (its access is atomicLoad, but WGSL
/// requires the storage variable to declare write access whenever
/// any entry of the pipeline writes through it — sharing the same
/// pipeline_layout between both pipelines).
pub(super) fn build_group_err_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_group_err_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub(super) fn build_cull_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<CullParams>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
        ],
    })
}

/// Two-binding pool BGL the cull pass uses (mesh_descriptors +
/// meshlets). Matches bindings 0-1 of [`crate::meshlet::pool::GpuGlobalMeshPool::bind_group_layout`]
/// so a single pool object can be sliced into either layout via
/// `bind_group_with_layout`-style construction at dispatch time.
pub(super) fn build_cull_pool_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let read_only = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_pool_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: read_only,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: read_only,
                count: None,
            },
        ],
    })
}

/// Cull BGL for the Hi-Z 2-pass entry. Drops binding 1 (`descriptors`,
/// per-mesh path) because the scene-pool Hi-Z entry never reads it,
/// and adds `culled_meshlets` (4) + `culled_count` (5). Bindings 0,
/// 2, 3 match `cull_bgl`. Total storage buffers in the pipeline layout:
/// 4 here + 2 in pool_bgl + 1 in scene_with_hi_z + 1 in group_err = 8,
/// exactly at the wgpu COMPUTE-stage limit.
pub(super) fn build_extended_cull_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let rw_storage = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_extended_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<CullParams>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: rw_storage,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: rw_storage,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
        ],
    })
}

/// Scene BGL extended with the previous-frame Hi-Z UBO (binding 2)
/// and pyramid texture (binding 3) for the Hi-Z 2-pass cull entry.
pub(super) fn build_scene_with_hi_z_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_scene_with_hi_z_bgl"),
        entries: &[
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
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(
                        std::mem::size_of::<SceneCullParams>() as u64,
                    ),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(
                        std::mem::size_of::<HiZTestParams>() as u64,
                    ),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

pub(super) fn build_hi_z_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_hi_z_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<HiZTestParams>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

/// Group 3 of the chunked cull (#1002): the per-group error arena the
/// #465 descent already used, plus the per-mesh bounding spheres the
/// instance pass tests and the chunk list it fills.
///
/// 🔴 One group, not two. `max_bind_groups` is six and the atomic cull
/// already spends five — a group of its own for the chunk list would
/// be the sixth and last. Binding 0 stays `group_max_err` so the two
/// meshlet-domain passes keep the layout they had.
///
/// `mesh_bounds` is the same buffer `lamp_cull` culls instances with —
/// spheres rather than boxes because they survive an arbitrary
/// rotation (#847).
pub(super) fn build_chunked_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let rw = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet_cull_chunked_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: rw,
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(4),
                },
                count: None,
            },
        ],
    })
}
