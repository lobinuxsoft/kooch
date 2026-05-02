//! Per-frame meshlet culling dispatcher.
//!
//! Owns the compute pipeline, per-frame [`CullParams`] UBO, the
//! visible-meshlet output buffer, and the atomic counter that doubles
//! as the indirect-draw `instance_count` source. One [`MeshletCull`]
//! is shared across frames; [`MeshletCull::dispatch`] is called once
//! per frame inside the render encoder, after camera matrices are
//! known.
//!
//! # Pipeline (PR-4 step 1)
//!
//! ```text
//! camera matrices  →  CullParams UBO          (CPU upload)
//!                          │
//!                          ▼
//!     reset(visible_count = 0)                (clear pass)
//!                          │
//!                          ▼
//!     dispatch cs_cull, ⌈meshlet_count/64⌉ workgroups
//!                          │
//!                          ▼
//!         visible_meshlets[0..visible_count]   (atomic-appended)
//! ```
//!
//! Indirect-draw arg writeback is wired in step 2 (next commit) — this
//! commit lays the buffer / bind-group / dispatch infrastructure that
//! step 2 then extends with binding(4) on the cull shader.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::cull::CullParams;
use super::gpu_meshlet::{meshlet_bind_group_layout, GpuMeshletMesh};

/// Indirect draw arguments laid out for `wgpu::RenderPass::draw_indirect`.
///
/// `vertex_count` is fixed at pipeline creation (one expanded triangle
/// fan per meshlet, see [`MeshletCull::vertex_count_per_instance`]).
/// `instance_count` is the only per-frame dynamic field — atomically
/// incremented by the cull shader once step 2 lands.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DrawIndirectArgs {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

const CULL_SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_cull.wgsl");

/// Owns one frame's worth of cull state. The output buffers (`visible_*`,
/// `indirect_args`) are sized at construction; recreate the dispatcher if
/// scene meshlet count grows past `capacity`.
pub struct MeshletCull {
    pipeline: wgpu::ComputePipeline,
    cull_bgl: wgpu::BindGroupLayout,
    meshlet_bgl: wgpu::BindGroupLayout,

    params_buffer: wgpu::Buffer,
    visible_meshlets: wgpu::Buffer,
    visible_count: wgpu::Buffer,
    indirect_args: wgpu::Buffer,

    capacity: u32,
    vertex_count_per_instance: u32,
}

impl MeshletCull {
    /// Storage capacity (in meshlets) of the visible-output buffer.
    /// Dispatching against a `GpuMeshletMesh` with more meshlets than
    /// this is a programmer error — the cull shader bounds-checks
    /// `meshlet_count`, so the excess are simply ignored.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Number of vertices the rasterizer fetches per meshlet instance.
    /// Equals `MAX_TRIANGLES * 3`; degenerate triangles (idx >=
    /// triangle_count) collapse to off-screen vertices in the meshlet
    /// vertex shader (PR-4 step 3).
    pub fn vertex_count_per_instance(&self) -> u32 {
        self.vertex_count_per_instance
    }

    /// `wgpu::Buffer` holding `[DrawIndirectArgs; 1]`. Bound as
    /// `BufferUsages::INDIRECT | STORAGE` so step 2 can also write it
    /// from the cull shader.
    pub fn indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.indirect_args
    }

    /// `wgpu::Buffer` holding `array<u32>` of meshlet ids that survived
    /// culling. Length is `visible_count` (read from the atomic). The
    /// rasterizer binds this and indexes by `@builtin(instance_index)`.
    pub fn visible_meshlets_buffer(&self) -> &wgpu::Buffer {
        &self.visible_meshlets
    }

    /// `wgpu::Buffer` holding `atomic<u32>` (single u32). Written by the
    /// cull shader, read back by tests, and copied into the indirect
    /// args' `instance_count` slot in step 2.
    pub fn visible_count_buffer(&self) -> &wgpu::Buffer {
        &self.visible_count
    }

    /// Bind group layout describing the cull shader's group(0).
    /// Re-exported so future passes (Hi-Z, backface) can extend it.
    pub fn cull_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.cull_bgl
    }

    /// Bind group layout describing the meshlet pool's group(1) — the
    /// rasterizer (PR-4 step 3) reuses the exact same handle so the
    /// cull and draw passes agree on storage-buffer slot numbering.
    pub fn meshlet_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.meshlet_bgl
    }

    /// Creates a dispatcher sized for at most `capacity` visible
    /// meshlets per frame. `max_triangles_per_meshlet` controls the
    /// fixed `vertex_count` used by the indirect draw — must match the
    /// builder's setting (default [`super::DEFAULT_MAX_TRIANGLES`]).
    pub fn new(
        device: &wgpu::Device,
        capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        assert!(capacity > 0, "MeshletCull capacity must be non-zero");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_cull_shader"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER_SOURCE.into()),
        });

        let cull_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_cull_bgl"),
            entries: &[
                // params (uniform)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<CullParams>() as u64,
                        ),
                    },
                    count: None,
                },
                // descriptors (storage, read)
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
                // visible_meshlets (storage, read_write)
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
                // visible_count (storage, atomic)
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
        });

        let meshlet_bgl = meshlet_bind_group_layout(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl), Some(&meshlet_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_params"),
            size: std::mem::size_of::<CullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let visible_meshlets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_ids"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let visible_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Indirect args: vertex_count is constant (set per-frame in
        // dispatch()); instance_count starts at zero and step 2 will
        // wire the cull shader to atomically increment it directly.
        // For now we keep `STORAGE | INDIRECT | COPY_DST` so step 2 is
        // a shader-only diff.
        let vertex_count_per_instance = max_triangles_per_meshlet * 3;
        let indirect_args = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_indirect_args"),
            contents: bytemuck::bytes_of(&DrawIndirectArgs {
                vertex_count: vertex_count_per_instance,
                instance_count: 0,
                first_vertex: 0,
                first_instance: 0,
            }),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            cull_bgl,
            meshlet_bgl,
            params_buffer,
            visible_meshlets,
            visible_count,
            indirect_args,
            capacity,
            vertex_count_per_instance,
        }
    }

    /// Dispatches the cull pass for `mesh` against `params`. Resets
    /// `visible_count` to zero before dispatch so each frame starts
    /// from a clean slate.
    ///
    /// The caller must keep `mesh` alive for the duration of the
    /// encoder submission — bind groups borrow its descriptor buffer.
    pub fn dispatch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMeshletMesh,
        params: &CullParams,
    ) {
        debug_assert!(
            mesh.meshlet_count <= self.capacity,
            "meshlet count {} exceeds dispatcher capacity {}",
            mesh.meshlet_count,
            self.capacity,
        );

        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));

        // Reset atomic counter. The shader appends with atomicAdd, so
        // any non-zero starting value would overflow the visible list
        // index space.
        encoder.clear_buffer(&self.visible_count, 0, None);

        let cull_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_cull_bg_dispatch"),
            layout: &self.cull_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: mesh.descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.visible_meshlets.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.visible_count.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meshlet_cull_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &cull_bg, &[]);
        let workgroups = mesh.meshlet_count.div_ceil(64);
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_indirect_args_layout_is_pod() {
        // Must match wgpu::DrawIndirectArgs exactly so we can write
        // straight into an INDIRECT-usage buffer.
        assert_eq!(std::mem::size_of::<DrawIndirectArgs>(), 16);
    }

    #[test]
    fn draw_indirect_args_default_is_zero() {
        let args = DrawIndirectArgs::default();
        assert_eq!(args.vertex_count, 0);
        assert_eq!(args.instance_count, 0);
        assert_eq!(args.first_vertex, 0);
        assert_eq!(args.first_instance, 0);
    }

    #[test]
    fn cull_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(CULL_SHADER_SOURCE)
            .expect("meshlet_cull.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_cull.wgsl should validate");
    }
}
