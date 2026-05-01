//! `GdfState` — wgpu-side ownership of the cascade-0 storage texture,
//! the populate compute pipeline, and the two bind groups (cascade-side
//! group 0 + pool-side group 1).
//!
//! See `gdf/mod.rs` for the bind-group split rationale.

use glam::Vec3;
use ome_bvh::AccelBuffers;
use wgpu::util::DeviceExt;

use super::POPULATE_SHADER_SOURCE;
use super::uniforms::{
    CASCADE_0_VOXEL_SIZE, CASCADE_0_VOXELS_PER_AXIS, CascadeDescriptor, POPULATE_WORKGROUP_XY,
    snap_to_voxel_grid,
};

/// Storage-texture format for cascade 0. `R32Float` is the only
/// single-channel float format wgpu 29 / WebGPU core advertises with
/// `STORAGE_BINDING` usage — see the gdf_populate.wgsl comment for the
/// adapter-feature exit. 64³ × 4 B = 1 MB constant per cascade.
const CASCADE_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Shader entry point in `gdf_populate.wgsl`.
const POPULATE_ENTRY_POINT: &str = "cs_populate";

/// Per-cascade GPU state. PR-3 holds cascade 0 only; the
/// `cascade_descriptor` is recomputed once per frame in
/// [`GdfState::dispatch_populate`] from the snapped camera position.
pub struct GdfState {
    cascade_texture: wgpu::Texture,
    cascade_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    uniforms_buffer: wgpu::Buffer,
    populate_pipeline: wgpu::ComputePipeline,
    populate_bg_layout_group0: wgpu::BindGroupLayout,
    populate_bg_layout_group1: wgpu::BindGroupLayout,
    populate_bg_group0: wgpu::BindGroup,
    populate_bg_group1: wgpu::BindGroup,
    last_descriptor: CascadeDescriptor,
}

impl GdfState {
    /// Build the cascade-0 storage texture, the populate pipeline, and
    /// both bind groups. The pool buffers are referenced through
    /// `accel_buffers` — pool buffers are pre-allocated once at
    /// `BvhState::new` and persistent thereafter, so a capacity grow
    /// would force a `rebind_pool_buffers` call (PR-5 territory; PR-3
    /// pool capacity is fixed).
    pub fn new(device: &wgpu::Device, accel_buffers: &AccelBuffers) -> Self {
        let cascade_texture = create_cascade_texture(device);
        let cascade_view = cascade_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ome_render::gdf::cascade_view_0"),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ome_render::gdf::cascade_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let initial_descriptor = CascadeDescriptor::cascade_0(Vec3::ZERO);
        let uniforms_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_render::gdf::cascade_uniforms"),
            contents: bytemuck::bytes_of(&initial_descriptor),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let populate_bg_layout_group0 = create_group0_layout(device);
        let populate_bg_layout_group1 = create_group1_layout(device);

        let populate_bg_group0 = create_group0_bind_group(
            device,
            &populate_bg_layout_group0,
            &uniforms_buffer,
            &cascade_view,
        );
        let populate_bg_group1 =
            create_group1_bind_group(device, &populate_bg_layout_group1, accel_buffers);

        let populate_pipeline = create_populate_pipeline(
            device,
            &populate_bg_layout_group0,
            &populate_bg_layout_group1,
        );

        Self {
            cascade_texture,
            cascade_view,
            sampler,
            uniforms_buffer,
            populate_pipeline,
            populate_bg_layout_group0,
            populate_bg_layout_group1,
            populate_bg_group0,
            populate_bg_group1,
            last_descriptor: initial_descriptor,
        }
    }

    /// Snap `camera_pos` to the cascade-0 voxel grid, write the
    /// resulting [`CascadeDescriptor`] to the uniform buffer, and
    /// dispatch the populate compute pass into `encoder`.
    ///
    /// PR-3 always re-dispatches (full rebuild every frame). Round-robin
    /// + dirty tracking is PR-5.
    pub fn dispatch_populate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera_pos: Vec3,
    ) {
        // Cascade origin = camera floored to the cascade voxel grid,
        // shifted so the camera sits at the cascade centre. The half-
        // extent shift keeps the camera inside the cascade as it walks
        // around — without it the camera would drift to a corner and
        // the cascade would only cover one octant.
        let half_extent =
            CASCADE_0_VOXEL_SIZE * (CASCADE_0_VOXELS_PER_AXIS as f32 * 0.5);
        let snapped_centre = snap_to_voxel_grid(camera_pos, CASCADE_0_VOXEL_SIZE);
        let world_origin = snapped_centre - Vec3::splat(half_extent);
        let descriptor = CascadeDescriptor::cascade_0(world_origin);
        queue.write_buffer(&self.uniforms_buffer, 0, bytemuck::bytes_of(&descriptor));
        self.last_descriptor = descriptor;

        let workgroups_per_axis = CASCADE_0_VOXELS_PER_AXIS / POPULATE_WORKGROUP_XY;
        let z_slabs = CASCADE_0_VOXELS_PER_AXIS;

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_render::gdf::populate_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.populate_pipeline);
        pass.set_bind_group(0, &self.populate_bg_group0, &[]);
        pass.set_bind_group(1, &self.populate_bg_group1, &[]);
        pass.dispatch_workgroups(workgroups_per_axis, workgroups_per_axis, z_slabs);
    }

    /// Rebuild the pool-side (group 1) bind group. Call when
    /// `accel_buffers` swaps out from under us — i.e. capacity grow
    /// (PR-5 territory).
    pub fn rebind_pool_buffers(&mut self, device: &wgpu::Device, accel_buffers: &AccelBuffers) {
        self.populate_bg_group1 = create_group1_bind_group(
            device,
            &self.populate_bg_layout_group1,
            accel_buffers,
        );
    }

    pub fn cascade_view(&self) -> &wgpu::TextureView {
        &self.cascade_view
    }

    pub fn cascade_texture(&self) -> &wgpu::Texture {
        &self.cascade_texture
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn uniforms_buffer(&self) -> &wgpu::Buffer {
        &self.uniforms_buffer
    }

    /// The descriptor written on the most recent dispatch — useful for
    /// debug overlays and integration tests that need to know the
    /// cascade origin without reading back the uniform buffer.
    pub fn last_descriptor(&self) -> CascadeDescriptor {
        self.last_descriptor
    }
}

fn create_cascade_texture(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ome_render::gdf::cascade_texture_0"),
        size: wgpu::Extent3d {
            width: CASCADE_0_VOXELS_PER_AXIS,
            height: CASCADE_0_VOXELS_PER_AXIS,
            depth_or_array_layers: CASCADE_0_VOXELS_PER_AXIS,
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

fn create_group0_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
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

/// Mirror of the layout `raymarch_pool_eval.wgsl` declares at
/// `@group(1) @binding(5..=10)`. Shared with the smoke harness's
/// `tests/pool_eval_smoke.rs` and the production raymarch fragment
/// path — the contract is fixed by the library shader.
fn create_group1_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
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

fn create_group0_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    cascade_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_render::gdf::populate_bg_group0"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(cascade_view),
            },
        ],
    })
}

fn create_group1_bind_group(
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

fn create_populate_pipeline(
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
