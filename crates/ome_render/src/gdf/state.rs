//! `GdfState` — wgpu-side ownership of the six GDF cascade textures,
//! the populate compute pipeline, and the per-cascade bind groups.
//!
//! See `gdf/mod.rs` for the bind-group split rationale.
//!
//! PR-5 of epic #370 fans cascade 0 out to six independent
//! cascades. Each cascade has its own `wgpu::Texture` + view +
//! per-dispatch uniform buffer (the descriptor the populate pass
//! reads). The fragment shader reads a single `GdfUniforms` array
//! covering all six. Round-robin scheduling lives in
//! [`super::scheduler::GdfScheduler`].

use glam::Vec3;
use ome_bvh::AccelBuffers;

use super::build::{
    create_cascade_sampler, create_cascade_texture, create_frag_uniforms_buffer,
    create_group0_bind_groups, create_group0_layout, create_group1_bind_group,
    create_group1_layout, create_populate_pipeline, create_populate_uniform_buffers,
};
use super::uniforms::{
    CASCADE_COUNT, CASCADE_VOXELS_PER_AXIS, CASCADE_VOXEL_SIZES, CascadeDescriptor,
    GdfUniforms, POPULATE_WORKGROUP_XY, snap_to_voxel_grid,
};

/// Per-cascade GPU state. Six cascades, each with a 64³ R32Float
/// storage texture + view + per-dispatch uniform; one shared sampler
/// + one shared `GdfUniforms` buffer for the fragment shader.
pub struct GdfState {
    cascade_textures: [wgpu::Texture; CASCADE_COUNT],
    cascade_views: [wgpu::TextureView; CASCADE_COUNT],
    sampler: wgpu::Sampler,
    populate_uniforms: [wgpu::Buffer; CASCADE_COUNT],
    frag_uniforms_buffer: wgpu::Buffer,
    populate_pipeline: wgpu::ComputePipeline,
    /// Group 0 layout retained so cascade-resize / format-swap paths
    /// can rebuild bind groups without re-deriving the layout.
    #[allow(dead_code)]
    populate_bg_layout_group0: wgpu::BindGroupLayout,
    populate_bg_layout_group1: wgpu::BindGroupLayout,
    populate_bg_group0: [wgpu::BindGroup; CASCADE_COUNT],
    populate_bg_group1: wgpu::BindGroup,
    last_descriptors: [CascadeDescriptor; CASCADE_COUNT],
    frag_uniforms_cache: GdfUniforms,
}

impl GdfState {
    /// Build the six cascade textures, the shared sampler + uniforms,
    /// the populate pipeline, and the per-cascade bind groups.
    pub fn new(device: &wgpu::Device, accel_buffers: &AccelBuffers) -> Self {
        let cascade_textures: [wgpu::Texture; CASCADE_COUNT] =
            std::array::from_fn(|c| create_cascade_texture(device, c));
        let cascade_views: [wgpu::TextureView; CASCADE_COUNT] =
            std::array::from_fn(|c| {
                cascade_textures[c].create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("ome_render::gdf::cascade_view_{c}")),
                    ..Default::default()
                })
            });
        let sampler = create_cascade_sampler(device);

        let populate_uniforms = create_populate_uniform_buffers(device);
        let frag_uniforms_buffer = create_frag_uniforms_buffer(device);

        let populate_bg_layout_group0 = create_group0_layout(device);
        let populate_bg_layout_group1 = create_group1_layout(device);

        let populate_bg_group0 = create_group0_bind_groups(
            device,
            &populate_bg_layout_group0,
            &populate_uniforms,
            &cascade_views,
        );
        let populate_bg_group1 =
            create_group1_bind_group(device, &populate_bg_layout_group1, accel_buffers);

        let populate_pipeline = create_populate_pipeline(
            device,
            &populate_bg_layout_group0,
            &populate_bg_layout_group1,
        );

        Self {
            cascade_textures,
            cascade_views,
            sampler,
            populate_uniforms,
            frag_uniforms_buffer,
            populate_pipeline,
            populate_bg_layout_group0,
            populate_bg_layout_group1,
            populate_bg_group0,
            populate_bg_group1,
            last_descriptors: std::array::from_fn(|c| {
                CascadeDescriptor::for_cascade(c, Vec3::ZERO)
            }),
            frag_uniforms_cache: GdfUniforms::from_origins(&[Vec3::ZERO; CASCADE_COUNT]),
        }
    }

    /// Snap `camera_pos` to cascade `c`'s voxel grid and dispatch its
    /// populate compute pass into `encoder`. Updates both
    /// `populate_uniforms[c]` (read by the compute shader) and the
    /// `c`-th entry of the fragment-side `GdfUniforms` (read by every
    /// future ray-march sample of that cascade).
    pub fn dispatch_populate_cascade(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        cascade_idx: usize,
        camera_pos: Vec3,
    ) {
        debug_assert!(cascade_idx < CASCADE_COUNT);
        let voxel_size = CASCADE_VOXEL_SIZES[cascade_idx];
        let half_extent = voxel_size * (CASCADE_VOXELS_PER_AXIS as f32 * 0.5);
        let snapped_centre = snap_to_voxel_grid(camera_pos, voxel_size);
        let world_origin = snapped_centre - Vec3::splat(half_extent);
        let descriptor = CascadeDescriptor::for_cascade(cascade_idx, world_origin);

        queue.write_buffer(
            &self.populate_uniforms[cascade_idx],
            0,
            bytemuck::bytes_of(&descriptor),
        );
        // Mirror the descriptor into the fragment-side `GdfUniforms`
        // so `pick_cascade` sees the new origin in the same frame.
        self.frag_uniforms_cache.cascades[cascade_idx] = descriptor;
        let offset = (cascade_idx * std::mem::size_of::<CascadeDescriptor>()) as u64;
        queue.write_buffer(
            &self.frag_uniforms_buffer,
            offset,
            bytemuck::bytes_of(&descriptor),
        );
        self.last_descriptors[cascade_idx] = descriptor;

        let workgroups_per_axis = CASCADE_VOXELS_PER_AXIS / POPULATE_WORKGROUP_XY;
        let z_slabs = CASCADE_VOXELS_PER_AXIS;

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&format!("ome_render::gdf::populate_pass_c{cascade_idx}")),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.populate_pipeline);
        pass.set_bind_group(0, &self.populate_bg_group0[cascade_idx], &[]);
        pass.set_bind_group(1, &self.populate_bg_group1, &[]);
        pass.dispatch_workgroups(workgroups_per_axis, workgroups_per_axis, z_slabs);
    }

    /// Backwards-compat helper: dispatch cascade 0 only. Tests that
    /// only need the finest cascade go through this path so they
    /// don't have to drive the round-robin scheduler.
    pub fn dispatch_populate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera_pos: Vec3,
    ) {
        self.dispatch_populate_cascade(encoder, queue, 0, camera_pos);
    }

    /// Rebuild the pool-side (group 1) bind group. Call when
    /// `accel_buffers` swaps out from under us — i.e. capacity grow.
    pub fn rebind_pool_buffers(&mut self, device: &wgpu::Device, accel_buffers: &AccelBuffers) {
        self.populate_bg_group1 = create_group1_bind_group(
            device,
            &self.populate_bg_layout_group1,
            accel_buffers,
        );
    }

    pub fn cascade_view(&self, c: usize) -> &wgpu::TextureView {
        &self.cascade_views[c]
    }

    /// All six cascade views in `[c=0, ..., c=5]` order. Consumed by
    /// the renderer's bind-group builder.
    pub fn cascade_views(&self) -> &[wgpu::TextureView; CASCADE_COUNT] {
        &self.cascade_views
    }

    pub fn cascade_texture(&self, c: usize) -> &wgpu::Texture {
        &self.cascade_textures[c]
    }

    /// Cascade 0 alias — kept stable for PR-3 / PR-4 tests pinned
    /// against the single-cascade public surface.
    pub fn cascade_texture_0(&self) -> &wgpu::Texture {
        &self.cascade_textures[0]
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Fragment-shader uniform buffer covering all six cascades (192 B).
    pub fn frag_uniforms_buffer(&self) -> &wgpu::Buffer {
        &self.frag_uniforms_buffer
    }

    /// Backwards-compat alias for PR-3/PR-4 tests that bound a single
    /// cascade descriptor uniform.
    pub fn uniforms_buffer(&self) -> &wgpu::Buffer {
        &self.frag_uniforms_buffer
    }

    /// Descriptor written on the most recent dispatch of cascade `c`.
    /// Useful for debug overlays + tests that need the snapped origin
    /// without reading back the GPU buffer.
    pub fn last_descriptor_for(&self, c: usize) -> CascadeDescriptor {
        self.last_descriptors[c]
    }

    /// Cascade-0 alias retained for the PR-3/PR-4 readback tests.
    pub fn last_descriptor(&self) -> CascadeDescriptor {
        self.last_descriptors[0]
    }
}
