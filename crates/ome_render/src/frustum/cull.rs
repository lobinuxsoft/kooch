//! [`FrustumCull`] — GPU compute that turns the shared BVH's
//! `leaf_aabbs` into a per-leaf indirect-draw array.

use bytemuck::{Pod, Zeroable};
use glam::Vec4;
use ome_bvh::SharedBvhState;

const SHADER_SOURCE: &str = include_str!("../../shaders/frustum_cull.wgsl");

/// Workgroup size matching the `@workgroup_size(64)` annotation in
/// `frustum_cull.wgsl`. Kept as a Rust constant so the dispatch math
/// can't drift from the shader.
const WORKGROUP_SIZE: u32 = 64;

/// Initial capacity (in leaves) for the indirect-args buffer. Grown
/// via `next_power_of_two` on demand; never shrunk — keeps capacity
/// churn bounded across scene-size changes.
const INITIAL_INDIRECT_CAPACITY: u32 = 256;

/// 6-plane frustum in `dot(plane.xyz, p) + plane.w >= 0` inside-half-
/// space form. Order is irrelevant to correctness; the engine uses
/// `[left, right, bottom, top, near, far]` by convention.
#[derive(Copy, Clone, Debug, Default)]
pub struct FrustumPlanes(pub [Vec4; 6]);

/// Mirrors `wgpu::util::DrawIndexedIndirectArgs` byte-for-byte (20 B,
/// std430-clean) so the compute shader can write directly into a
/// buffer the mesh pass binds via `draw_indexed_indirect`. Re-declared
/// here (instead of re-exporting wgpu's) because wgpu's type isn't
/// `bytemuck::Pod`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq, Eq)]
pub struct DrawIndexedIndirectArgs {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

/// Per-frame uniform read by `frustum_cull.wgsl`. 112 B std140 — the
/// 6-plane array plus 16 B of trailing scalars (n + uniform mesh
/// index count + 8 B padding to keep the next-write-aligned).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct FrustumUniforms {
    /// Plane equations in `(n.x, n.y, n.z, d)` form, inside half-
    /// space `dot(n, p) + d >= 0`.
    pub planes: [[f32; 4]; 6],
    /// Number of leaves the dispatch should process — threads with
    /// `gid.x >= n` early-out.
    pub n: u32,
    /// Index count of the mesh shared by every IS_VISIBLE_MESH leaf
    /// in this dispatch. S5 ships uniform-mesh assumption; per-leaf
    /// mesh metadata is a follow-up when the renderer maintains a
    /// proper mesh atlas.
    pub index_count_per_mesh: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

impl Default for FrustumUniforms {
    fn default() -> Self {
        Self {
            planes: [[0.0; 4]; 6],
            n: 0,
            index_count_per_mesh: 0,
            _pad0: 0,
            _pad1: 0,
        }
    }
}

/// GPU-driven frustum cull. Owns the compute pipeline, the uniform
/// buffer, and the indirect-args output buffer the mesh pass consumes.
///
/// Re-binds the BVH's `leaf_aabbs_buffer` lazily — when
/// [`SharedBvhState::current_slot_index`] flips, [`Self::cull`] rebuilds
/// the bind group so the dispatch reads the just-resolved slot's data.
pub struct FrustumCull {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    indirect_buffer: wgpu::Buffer,
    indirect_capacity: u32,
    bind_group: Option<wgpu::BindGroup>,
    /// Slot index the cached `bind_group` was built against. `None`
    /// before the first cull dispatch.
    bound_slot: Option<u8>,
    /// Indirect capacity the cached `bind_group` was built against.
    /// Used to detect "indirect buffer was reallocated" alongside slot
    /// changes.
    bound_indirect_capacity: Option<u32>,
}

impl FrustumCull {
    /// Build the compute pipeline + initial uniform / indirect buffers.
    /// `pipeline_cache` amortises shader compile time across crates if
    /// the engine's shared cache is plumbed in.
    pub fn new(device: &wgpu::Device, pipeline_cache: Option<&wgpu::PipelineCache>) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frustum_cull_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frustum_cull_bgl"),
            entries: &[
                // leaf_aabbs (read)
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
                // visible_indirect (read_write storage)
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
                // frustum uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frustum_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("frustum_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("frustum_cull_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: pipeline_cache,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frustum_cull_uniforms"),
            size: std::mem::size_of::<FrustumUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let indirect_buffer = make_indirect_buffer(device, INITIAL_INDIRECT_CAPACITY);

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            indirect_buffer,
            indirect_capacity: INITIAL_INDIRECT_CAPACITY,
            bind_group: None,
            bound_slot: None,
            bound_indirect_capacity: None,
        }
    }

    /// Borrow the indirect-args buffer the mesh pass consumes via
    /// `draw_indexed_indirect`. One `DrawIndexedIndirectArgs` per leaf
    /// in original input order; entries past `bvh.current_n()` carry
    /// undefined contents (the dispatch did not write to them) — the
    /// mesh pass must clamp its draw count to `current_n()`.
    pub fn indirect_buffer(&self) -> &wgpu::Buffer {
        &self.indirect_buffer
    }

    /// Number of indirect-args slots currently allocated. Mesh pass
    /// callers comparing against `bvh.current_n()` assert this is at
    /// least that large after a `cull` returned without growing past
    /// it.
    pub fn indirect_capacity(&self) -> u32 {
        self.indirect_capacity
    }

    /// Run the frustum cull compute pass. No-op when the BVH has not
    /// resolved its first build (`bvh.current_n() == 0`); otherwise
    /// dispatches `ceil(n / 64)` workgroups and writes one indirect
    /// args entry per leaf.
    ///
    /// `index_count_per_mesh` is folded into every emitted args entry
    /// — S5 ships the uniform-mesh assumption (every IS_VISIBLE_MESH
    /// leaf draws the same indexed mesh). When the engine grows a
    /// proper mesh atlas with per-entity index counts, this argument
    /// becomes a per-leaf metadata buffer.
    pub fn cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        bvh: &SharedBvhState,
        planes: &FrustumPlanes,
        index_count_per_mesh: u32,
    ) {
        let n = bvh.current_n();
        if n == 0 {
            return;
        }

        // Grow indirect buffer to at least n entries.
        if n > self.indirect_capacity {
            let new_cap = n.next_power_of_two().max(INITIAL_INDIRECT_CAPACITY);
            self.indirect_buffer = make_indirect_buffer(device, new_cap);
            self.indirect_capacity = new_cap;
            // Capacity changed — invalidate bind group so it rebinds
            // against the new buffer.
            self.bind_group = None;
        }

        // Upload uniforms.
        let uniforms = FrustumUniforms {
            planes: planes.0.map(|v| v.to_array()),
            n,
            index_count_per_mesh,
            _pad0: 0,
            _pad1: 0,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Rebuild bind group when the BVH swapped slots or our
        // indirect buffer was reallocated.
        let slot = bvh.current_slot_index();
        let bg_stale = self.bind_group.is_none()
            || self.bound_slot != Some(slot)
            || self.bound_indirect_capacity != Some(self.indirect_capacity);
        if bg_stale {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frustum_cull_bg"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: bvh.current_leaf_aabbs().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.indirect_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                ],
            }));
            self.bound_slot = Some(slot);
            self.bound_indirect_capacity = Some(self.indirect_capacity);
        }

        let bind_group = self
            .bind_group
            .as_ref()
            .expect("bind group rebuilt this frame");

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("frustum_cull_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        let workgroups = n.div_ceil(WORKGROUP_SIZE);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
}

fn make_indirect_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frustum_cull_indirect"),
        size: capacity as u64 * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
