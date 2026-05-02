//! GDF wgpu resource construction — texture / view / sampler / bind
//! group factories used by [`super::state::GdfState`]. Split out of
//! `state.rs` so that file stays under the 400-LoC monolithic
//! threshold once PR-5 of epic #370 fans the cascade out to six
//! independent levels.
//!
//! The shapes are pinned by tests in `gdf/uniforms.rs` and the
//! integration tests in `tests/gdf_*`. Visibility is `pub(super)` —
//! only `GdfState` consumes these factories.

use ome_bvh::AccelBuffers;
use wgpu::util::DeviceExt;

use super::POPULATE_SHADER_SOURCE;
use super::uniforms::{
    CASCADE_COUNT, CASCADE_VOXELS_PER_AXIS, CascadeDescriptor, GdfUniforms,
};

/// Storage-texture format for every cascade. `R32Float` is the only
/// single-channel float format wgpu 29 / WebGPU core advertises with
/// `STORAGE_BINDING` usage. 64³ × 4 B = 1 MB per cascade; six
/// cascades = 6 MB GDF VRAM total.
pub(super) const CASCADE_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Shader entry point inside `gdf_populate.wgsl`.
pub(super) const POPULATE_ENTRY_POINT: &str = "cs_populate";

/// Build one R32Float 3D storage + sampleable cascade texture.
pub(super) fn create_cascade_texture(device: &wgpu::Device, cascade_idx: usize) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("ome_render::gdf::cascade_texture_{cascade_idx}")),
        size: wgpu::Extent3d {
            width: CASCADE_VOXELS_PER_AXIS,
            height: CASCADE_VOXELS_PER_AXIS,
            depth_or_array_layers: CASCADE_VOXELS_PER_AXIS,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: CASCADE_TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Single shared sampler — clamp-to-edge linear, mirrors the PR-4
/// configuration. All six cascade views read through it.
pub(super) fn create_cascade_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ome_render::gdf::cascade_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// Per-cascade populate uniform: a single `CascadeDescriptor` (32 B)
/// the populate dispatch reads to know its cascade origin + voxel
/// size. Six independent buffers so all six dispatches can land in
/// the same encoder without trampling each other's uniform writes.
pub(super) fn create_populate_uniform_buffers(
    device: &wgpu::Device,
) -> [wgpu::Buffer; CASCADE_COUNT] {
    std::array::from_fn(|c| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("ome_render::gdf::populate_uniforms_{c}")),
            contents: bytemuck::bytes_of(&CascadeDescriptor::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    })
}

/// Fragment-shader uniform: the full `[CascadeDescriptor; 6]` array
/// (192 B). The host rewrites a single descriptor when its cascade
/// snaps to a new origin, but the GPU sees one stable buffer.
pub(super) fn create_frag_uniforms_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ome_render::gdf::frag_uniforms"),
        contents: bytemuck::bytes_of(&GdfUniforms::default()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Group 0 of the populate pipeline — per-dispatch cascade descriptor
/// + the storage 3D texture for the target cascade.
pub(super) fn create_group0_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_render::gdf::populate_bgl_group0"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: CASCADE_TEXTURE_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D3,
                },
                count: None,
            },
        ],
    })
}

/// Group 1 of the populate pipeline — the OmeAccel pool buffers
/// (bindings 5..=10), shared with `raymarch_pool_eval.wgsl`.
pub(super) fn create_group1_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ome_render::gdf::populate_bgl_group1"),
        entries: &[
            entry(5, storage),  // tlas_nodes
            entry(6, storage),  // chunk_descriptors
            entry(7, storage),  // bvh_nodes_pool
            entry(8, storage),  // leaf_aabbs_pool
            entry(9, storage),  // primitives_pool
            entry(10, uniform), // tlas_uniforms
        ],
    })
}

/// Build the six per-cascade group-0 bind groups (one per cascade
/// uniform / storage texture pair).
pub(super) fn create_group0_bind_groups(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    populate_uniforms: &[wgpu::Buffer; CASCADE_COUNT],
    cascade_views: &[wgpu::TextureView; CASCADE_COUNT],
) -> [wgpu::BindGroup; CASCADE_COUNT] {
    std::array::from_fn(|c| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("ome_render::gdf::populate_bg_group0_c{c}")),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: populate_uniforms[c].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&cascade_views[c]),
                },
            ],
        })
    })
}

/// Build the (shared) group-1 bind group from the OmeAccel pool.
pub(super) fn create_group1_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    pool: &AccelBuffers,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_render::gdf::populate_bg_group1"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 5, resource: pool.tlas_nodes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: pool.chunk_descriptors.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: pool.bvh_nodes_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: pool.leaf_aabbs_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 9, resource: pool.primitives_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: pool.tlas_uniforms.as_entire_binding() },
        ],
    })
}

/// Compile + link the populate compute pipeline. Pipeline layout
/// captures both bind-group layouts so a single pipeline drives all
/// six cascade dispatches.
pub(super) fn create_populate_pipeline(
    device: &wgpu::Device,
    layout_group0: &wgpu::BindGroupLayout,
    layout_group1: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ome_render::gdf::populate_module"),
        source: wgpu::ShaderSource::Wgsl(POPULATE_SHADER_SOURCE.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ome_render::gdf::populate_pipeline_layout"),
        bind_group_layouts: &[Some(layout_group0), Some(layout_group1)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ome_render::gdf::populate_pipeline"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(POPULATE_ENTRY_POINT),
        compilation_options: Default::default(),
        cache: None,
    })
}
