//! Bind-group + layout factories for the production raymarch
//! pipeline. Split out of `renderer.rs` so that file stays under the
//! 400-LoC monolithic threshold once PR-5 of epic #370 added six
//! GDF cascade texture entries to group 1.
//!
//! Visibility is `pub(super)` — only `RayMarchRenderer::new` and
//! `make_camera_bg` consumers reach in here.

use crate::gdf::GdfState;

/// Camera-side group 0 bind group: `(0)` `CameraUniforms`
/// + `(1)` `RayMarchParams`.
pub(super) fn make_camera_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera: &wgpu::Buffer,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("raymarch_camera_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
        ],
    })
}

/// Bind-group layout entries for the pool-driven scene bind group.
/// Group 1:
///   - `(0)` `scene_meta` uniform.
///   - `(5..=10)` OmeAccel pool buffers (PR-2 of #360). Kept for PR-8
///     hybrid surface refinement; naga prunes them out of the live
///     raymarch pipeline since `eval_scene_bvh` no longer descends.
///   - `(11..=16)` six GDF cascade textures (PR-5 of epic #370).
///   - `(17)` shared cascade sampler (clamp-to-edge linear).
///   - `(18)` `GdfUniforms` (`array<CascadeDescriptor, 6>`, 192 B).
pub(super) fn pool_scene_bgl_entries() -> [wgpu::BindGroupLayoutEntry; 15] {
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
    let cascade_texture = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D3,
        multisampled: false,
    };
    let cascade_sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    [
        entry(0, uniform),          // scene_meta
        entry(5, storage),          // tlas_nodes
        entry(6, storage),          // chunk_descriptors
        entry(7, storage),          // bvh_nodes_pool
        entry(8, storage),          // leaf_aabbs_pool
        entry(9, storage),          // primitives_pool
        entry(10, uniform),         // tlas_uniforms
        entry(11, cascade_texture), // gdf_cascade_0
        entry(12, cascade_texture), // gdf_cascade_1
        entry(13, cascade_texture), // gdf_cascade_2
        entry(14, cascade_texture), // gdf_cascade_3
        entry(15, cascade_texture), // gdf_cascade_4
        entry(16, cascade_texture), // gdf_cascade_5
        entry(17, cascade_sampler), // gdf_sampler
        entry(18, uniform),         // gdf_uniforms (GdfUniforms)
    ]
}

/// Build the pool-driven scene bind group. Pool buffers come from
/// `OmeAccel::buffers()` — pre-allocated at `BvhState::new`, never
/// reallocated. Six GDF cascade views + sampler + the multi-cascade
/// uniform buffer come from `GdfState`, also stable for the
/// renderer's lifetime. Built ONCE at construction.
pub(super) fn make_pool_scene_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    meta: &wgpu::Buffer,
    pool: &ome_bvh::AccelBuffers,
    gdf: &GdfState,
) -> wgpu::BindGroup {
    let views = gdf.cascade_views();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("raymarch_scene_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: meta.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: pool.tlas_nodes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: pool.chunk_descriptors.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: pool.bvh_nodes_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: pool.leaf_aabbs_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 9, resource: pool.primitives_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: pool.tlas_uniforms.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&views[0]) },
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&views[1]) },
            wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(&views[2]) },
            wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&views[3]) },
            wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&views[4]) },
            wgpu::BindGroupEntry { binding: 16, resource: wgpu::BindingResource::TextureView(&views[5]) },
            wgpu::BindGroupEntry { binding: 17, resource: wgpu::BindingResource::Sampler(gdf.sampler()) },
            wgpu::BindGroupEntry { binding: 18, resource: gdf.frag_uniforms_buffer().as_entire_binding() },
        ],
    })
}
