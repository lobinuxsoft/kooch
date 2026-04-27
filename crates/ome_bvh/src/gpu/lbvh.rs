//! Karras LBVH constructor — three-pass GPU pipeline that turns a
//! sorted Morton + AABB list into a 2N-1 [`BvhNode`] array
//! byte-identical to the CPU [`Bvh::build`] output.
//!
//! Passes (each is its own dispatch on the same encoder):
//!
//! 1. **`write_leaves`** (`karras_leaves.wgsl`) — N threads, each
//!    writes its leaf into `nodes[(N-1) + k]`. Reads
//!    `original_aabbs[sorted_indices[k]]` (Morton-permuted lookup).
//!    Sets `done[leaf_idx] = 1` so the propagation pass knows the
//!    leaf's AABB is finalised.
//! 2. **`karras_internal`** (`karras_internal.wgsl`) — N-1 threads,
//!    one per internal node. Each runs Karras' algorithm on the
//!    sorted Morton codes to determine its range, split, and
//!    children, then writes `nodes[i]`, the parent pointers for its
//!    two children, and `done[i] = 0`.
//! 3. **`aabb_propagate`** (`karras_aabb.wgsl`) — looped on the host
//!    once per tree level (⌈log₂ N⌉ + slack iterations). Each
//!    dispatch finalises every internal whose children were finalised
//!    in earlier dispatches. The dispatch boundary is the only
//!    portable cross-workgroup memory barrier in WGSL — atomics give
//!    ordering on the atomic itself but not on adjacent memory, so
//!    the single-dispatch atomic-counter approach Karras' CUDA
//!    implementation uses (relying on `__threadfence`) does not work
//!    portably.
//!
//! All three pipelines are owned by [`LbvhPipelines`]; reusable GPU
//! buffers live in [`LbvhBuffers`]. The high-level orchestration
//! [`dispatch_lbvh_build`] wires them together with the existing
//! Morton + onesweep sort outputs.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::node::BvhNode;

/// Workgroup size for the three Karras passes. Matches the
/// `@workgroup_size(64)` declarations in the WGSL files. 64 is the
/// AMD wavefront size and an even divisor of NVIDIA / Intel
/// subgroup widths — portable choice with no per-vendor tuning.
const LBVH_WORKGROUP_SIZE: u32 = 64;

/// Initial capacity for the LBVH buffers (in leaves count). Grows by
/// `next_power_of_two` when an upload exceeds capacity.
const INITIAL_LBVH_CAPACITY: u64 = 1024;

/// Slack added on top of `⌈log₂ N⌉` for the AABB propagation loop.
/// Karras' index tie-break keeps tree depth bounded by `log₂ N`
/// even under fully-duplicate Morton codes — slack absorbs any
/// residual unbalance and keeps the loop count comfortably above
/// the worst observed depth.
const AABB_ITERATION_SLACK: u32 = 4;

/// Uniform configuration for every Karras pass — only the leaf count
/// `n` is dynamic; padding completes the 16-byte std140/std430 slot.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct LbvhConfig {
    pub n: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// Compiled compute pipelines for the three LBVH passes. Built once
/// per `BvhGpuBuilder`, reused across builds. `pipeline_cache`
/// integration is wired through the constructor.
pub struct LbvhPipelines {
    pub leaves_pipeline: wgpu::ComputePipeline,
    pub leaves_bgl: wgpu::BindGroupLayout,
    pub internal_pipeline: wgpu::ComputePipeline,
    pub internal_bgl: wgpu::BindGroupLayout,
    pub aabb_pipeline: wgpu::ComputePipeline,
    pub aabb_bgl: wgpu::BindGroupLayout,
}

/// Reusable GPU buffers for the LBVH build. Grow on demand, never
/// realloc'd per build (per the production-from-day-1 rule).
pub struct LbvhBuffers {
    /// Output flat tree, `2N-1` [`BvhNode`] entries.
    pub nodes_buffer: wgpu::Buffer,
    /// Parent index for every node (`2N-1` entries). Written by the
    /// internal pass; reserved for future GPU traversal kernels (PR-4
    /// raymarch culling, PR-5 collision broadphase). Not read by the
    /// AABB propagation pass.
    pub parents_buffer: wgpu::Buffer,
    /// `done[node]` flag (`2N-1` entries) — leaves are finalised by
    /// pass 1, internals by pass 3 once their two children are done.
    pub done_buffer: wgpu::Buffer,
    /// Capacity in *leaves count* (N).
    pub capacity: u64,
    /// Uniform with `n` for every dispatch — written via
    /// `queue.write_buffer` at the start of each build.
    pub config_buffer: wgpu::Buffer,
}

impl LbvhPipelines {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        // -- write_leaves pipeline (5 bindings: nodes, aabbs, indices, done, cfg) --
        let leaves_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_leaves"),
            source: wgpu::ShaderSource::Wgsl(super::KARRAS_LEAVES_WGSL.into()),
        });
        let leaves_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_leaves_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let leaves_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_leaves_pl"),
            bind_group_layouts: &[Some(&leaves_bgl)],
            immediate_size: 0,
        });
        let leaves_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_leaves_pipeline"),
            layout: Some(&leaves_pl),
            module: &leaves_shader,
            entry_point: Some("write_leaves_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- karras_internal pipeline (5 bindings: nodes, morton, parents, done, cfg) --
        let internal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_internal"),
            source: wgpu::ShaderSource::Wgsl(super::KARRAS_INTERNAL_WGSL.into()),
        });
        let internal_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_internal_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let internal_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_internal_pl"),
            bind_group_layouts: &[Some(&internal_bgl)],
            immediate_size: 0,
        });
        let internal_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_internal_pipeline"),
            layout: Some(&internal_pl),
            module: &internal_shader,
            entry_point: Some("karras_internal_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- aabb_propagate pipeline (3 bindings: nodes, done, cfg) --
        let aabb_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::karras_aabb"),
            source: wgpu::ShaderSource::Wgsl(super::KARRAS_AABB_WGSL.into()),
        });
        let aabb_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::karras_aabb_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<BvhNode>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let aabb_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::karras_aabb_pl"),
            bind_group_layouts: &[Some(&aabb_bgl)],
            immediate_size: 0,
        });
        let aabb_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::karras_aabb_pipeline"),
            layout: Some(&aabb_pl),
            module: &aabb_shader,
            entry_point: Some("aabb_propagate_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        Self {
            leaves_pipeline,
            leaves_bgl,
            internal_pipeline,
            internal_bgl,
            aabb_pipeline,
            aabb_bgl,
        }
    }
}

impl LbvhBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let nodes_buffer = make_nodes_buffer(device, INITIAL_LBVH_CAPACITY);
        let parents_buffer = make_aux_u32_buffer(device, "lbvh_parents", INITIAL_LBVH_CAPACITY);
        let done_buffer = make_aux_u32_buffer(device, "lbvh_done", INITIAL_LBVH_CAPACITY);
        let config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::lbvh_config"),
            contents: bytemuck::bytes_of(&LbvhConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            nodes_buffer,
            parents_buffer,
            done_buffer,
            capacity: INITIAL_LBVH_CAPACITY,
            config_buffer,
        }
    }

    /// Grow the LBVH buffers if `n_leaves` exceeds the current
    /// capacity. New capacity is `next_power_of_two(n_leaves)`.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, n_leaves: u64) {
        if n_leaves <= self.capacity {
            return;
        }
        let new_cap = n_leaves.next_power_of_two().max(INITIAL_LBVH_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.parents_buffer = make_aux_u32_buffer(device, "lbvh_parents", new_cap);
        self.done_buffer = make_aux_u32_buffer(device, "lbvh_done", new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
    // 2N-1 nodes — round to 2N for capacity arithmetic simplicity.
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::lbvh_nodes"),
        size: 2 * n * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_aux_u32_buffer(device: &wgpu::Device, suffix: &str, n: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("ome_bvh::{suffix}")),
        size: 2 * n * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Number of times the AABB propagation pass must be dispatched to
/// guarantee every internal node converges. Karras' index tie-break
/// keeps tree depth bounded by `⌈log₂ N⌉` regardless of input
/// (duplicates fall back to the index-bit prefix); the slack absorbs
/// any residual non-canonical depth from numerical edge cases.
fn aabb_iterations(n: u32) -> u32 {
    if n <= 1 {
        return 0;
    }
    // Bits required to represent (n - 1) — i.e. ⌈log₂ n⌉.
    let log_n = 32 - (n - 1).leading_zeros();
    log_n + AABB_ITERATION_SLACK
}

/// Pass 1 of the Karras build: write the N leaves into
/// `nodes[(N-1)..(2N-1))` and set `done[leaf_idx] = 1`. Safe for any
/// `n >= 1`. The orchestrator calls this unconditionally and then
/// branches on `n > 1` for the internal + AABB passes — keeping the
/// control flow visible at the call site (and avoiding the
/// `workgroup_count = 0` UB risk that some backends exhibit when
/// dispatched on the internal pass with `n == 1`).
pub fn dispatch_lbvh_leaves_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    original_aabbs: &wgpu::Buffer,
    sorted_indices: &wgpu::Buffer,
    n: u32,
) {
    if n == 0 {
        return;
    }

    // The config uniform is shared with the internal + aabb passes; we
    // write it once here so subsequent passes pick up the same `n`.
    let cfg = LbvhConfig {
        n,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
    };
    queue.write_buffer(&buffers.config_buffer, 0, bytemuck::bytes_of(&cfg));

    let leaves_workgroups = n.div_ceil(LBVH_WORKGROUP_SIZE);

    let leaves_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_leaves_bg"),
        layout: &pipelines.leaves_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: original_aabbs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: sorted_indices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::karras_leaves_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.leaves_pipeline);
    pass.set_bind_group(0, &leaves_bg, &[]);
    pass.dispatch_workgroups(leaves_workgroups.max(1), 1, 1);
}

/// Passes 2+3 of the Karras build: parallel internal-node construction
/// followed by `⌈log₂ n⌉ + slack` AABB propagation dispatches.
/// **Requires `n >= 2`** — the caller must guard against the trivial
/// 1-leaf case (where there are no internals to build).
pub fn dispatch_lbvh_internal_and_aabb_into(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    sorted_morton: &wgpu::Buffer,
    n: u32,
) {
    debug_assert!(
        n >= 2,
        "dispatch_lbvh_internal_and_aabb_into requires n >= 2 — \
         orchestrator must skip this for n in {{0, 1}}"
    );

    let internal_workgroups = (n - 1).div_ceil(LBVH_WORKGROUP_SIZE);

    let internal_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_internal_bg"),
        layout: &pipelines.internal_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: sorted_morton.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.parents_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::karras_internal_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.internal_pipeline);
        pass.set_bind_group(0, &internal_bg, &[]);
        pass.dispatch_workgroups(internal_workgroups.max(1), 1, 1);
    }

    // -- pass 3: aabb propagation, looped one tree level per dispatch --
    let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_aabb_bg"),
        layout: &pipelines.aabb_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    let iterations = aabb_iterations(n);
    for iter in 0..iterations {
        let label = format!("ome_bvh::karras_aabb_pass_{iter}");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.aabb_pipeline);
        pass.set_bind_group(0, &aabb_bg, &[]);
        pass.dispatch_workgroups(internal_workgroups.max(1), 1, 1);
    }
}

/// Convenience wrapper that dispatches the full Karras build (leaves +
/// internal + aabb) for any `n >= 0`. Used by the existing per-stage
/// integration tests; production code in [`crate::gpu::build`] calls
/// the two halves explicitly so the `n == 1` branch is visible.
///
/// Inputs (caller-owned buffers; not modified by this function):
/// - `original_aabbs`: storage buffer of [`crate::gpu::types::GpuAabb`]
///   in the *original* (pre-sort) order. Used by `write_leaves` via
///   the indirection through `sorted_indices`.
/// - `sorted_morton`: storage buffer of `u32` Morton codes after the
///   onesweep sort (length `n`).
/// - `sorted_indices`: storage buffer of `u32` payload indices after
///   the onesweep sort (length `n`). `sorted_indices[k]` is the
///   *original* index of the item at Morton-sorted position `k`.
///
/// Output: `buffers.nodes_buffer` populated with `2n-1` BvhNodes
/// byte-identical to `Bvh::build(items).nodes`.
pub fn dispatch_lbvh_build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    original_aabbs: &wgpu::Buffer,
    sorted_morton: &wgpu::Buffer,
    sorted_indices: &wgpu::Buffer,
    n: u32,
) {
    if n == 0 {
        return;
    }
    dispatch_lbvh_leaves_into(
        device,
        queue,
        encoder,
        pipelines,
        buffers,
        original_aabbs,
        sorted_indices,
        n,
    );
    if n >= 2 {
        dispatch_lbvh_internal_and_aabb_into(
            device,
            queue,
            encoder,
            pipelines,
            buffers,
            sorted_morton,
            n,
        );
    }
}

/// Test-only readback of the nodes buffer. Returns `2n-1` BvhNodes
/// in a CPU `Vec`. Production code never roundtrips this — the GPU
/// builder owns the buffer for downstream traversal kernels.
#[cfg(test)]
pub fn readback_nodes_for_test(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffers: &LbvhBuffers,
    n: u32,
) -> Vec<BvhNode> {
    let total = (2 * n - 1) as u64;
    let bytes = total * std::mem::size_of::<BvhNode>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::lbvh_nodes_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::lbvh_nodes_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(&buffers.nodes_buffer, 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll failed");
    receiver
        .recv()
        .expect("map_async sender dropped")
        .expect("map_async failed");
    let data = slice.get_mapped_range();
    let v: Vec<BvhNode> = bytemuck::cast_slice::<u8, BvhNode>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aabb::Aabb;
    use crate::bvh::Bvh;
    use crate::gpu::builder::test_device;
    use crate::gpu::types::GpuAabb;
    use crate::morton::MortonCode;
    use glam::Vec3;
    use wgpu::util::DeviceExt;

    /// Replicate the CPU's morton encoding + stable sort to produce
    /// `(sorted_morton, sorted_indices, original_aabbs)` triples for
    /// the GPU dispatch. Decouples LBVH testing from sort correctness.
    fn cpu_prepare_inputs(
        aabbs: &[Aabb],
    ) -> (Vec<u32>, Vec<u32>, Vec<GpuAabb>) {
        let scene = aabbs.iter().fold(Aabb::EMPTY, |acc, a| acc.union(a));
        let extent = scene.max - scene.min;
        let inv = Vec3::new(
            if extent.x > 0.0 { 1.0 / extent.x } else { 0.0 },
            if extent.y > 0.0 { 1.0 / extent.y } else { 0.0 },
            if extent.z > 0.0 { 1.0 / extent.z } else { 0.0 },
        );
        let mut indexed: Vec<(MortonCode, u32)> = aabbs
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let normalized = (a.center() - scene.min) * inv;
                (MortonCode::from_normalized(normalized), i as u32)
            })
            .collect();
        indexed.sort_by_key(|(c, _)| *c);
        let sorted_morton: Vec<u32> = indexed.iter().map(|(c, _)| c.0).collect();
        let sorted_indices: Vec<u32> = indexed.iter().map(|(_, i)| *i).collect();
        let originals: Vec<GpuAabb> = aabbs.iter().copied().map(GpuAabb::from).collect();
        (sorted_morton, sorted_indices, originals)
    }

    fn upload_storage<T: bytemuck::Pod>(
        device: &wgpu::Device,
        data: &[T],
        label: &str,
    ) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn run_gpu_lbvh(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aabbs: &[Aabb],
    ) -> Vec<BvhNode> {
        let n = aabbs.len() as u32;
        let pipelines = LbvhPipelines::new(device, None);
        let mut buffers = LbvhBuffers::new(device);
        buffers.ensure_capacity(device, n as u64);

        let (sorted_morton, sorted_indices, originals) = cpu_prepare_inputs(aabbs);
        let morton_buf = upload_storage(device, &sorted_morton, "test_sorted_morton");
        let indices_buf = upload_storage(device, &sorted_indices, "test_sorted_indices");
        let aabbs_buf = upload_storage(device, &originals, "test_original_aabbs");

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::test_lbvh_encoder"),
        });
        dispatch_lbvh_build(
            device,
            queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &aabbs_buf,
            &morton_buf,
            &indices_buf,
            n,
        );
        queue.submit(std::iter::once(encoder.finish()));
        readback_nodes_for_test(device, queue, &buffers, n)
    }

    fn aabb_at(centre: Vec3, half: f32) -> Aabb {
        Aabb::from_centre(centre, Vec3::splat(half))
    }

    fn assert_gpu_matches_cpu(gpu: &[BvhNode], cpu: &[BvhNode]) {
        if gpu == cpu {
            return;
        }
        assert_eq!(gpu.len(), cpu.len(), "node count mismatch");
        for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
            if g != c {
                panic!(
                    "GPU/CPU diverge at node[{i}]:\n  gpu: {:?}\n  cpu: {:?}",
                    g, c
                );
            }
        }
    }

    #[test]
    fn aabb_iterations_grows_with_log_n() {
        assert_eq!(aabb_iterations(0), 0);
        assert_eq!(aabb_iterations(1), 0);
        // ⌈log₂ 2⌉ = 1, + 4 slack.
        assert_eq!(aabb_iterations(2), 5);
        assert_eq!(aabb_iterations(8), 3 + AABB_ITERATION_SLACK);
        assert_eq!(aabb_iterations(1024), 10 + AABB_ITERATION_SLACK);
        assert_eq!(aabb_iterations(65536), 16 + AABB_ITERATION_SLACK);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_single_leaf() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("ome_bvh::gpu::lbvh: no GPU adapter — skipping");
            return;
        };
        let aabbs = vec![aabb_at(Vec3::ZERO, 1.0)];
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_two_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs = vec![
            aabb_at(Vec3::ZERO, 0.5),
            aabb_at(Vec3::splat(10.0), 0.5),
        ];
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_8_balanced_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..8)
            .map(|i| aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_5_leaves_asymmetric() {
        // Asymmetric tree — exercises the "right child isn't left + 1"
        // case where one subtree is internal, the other a leaf.
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..5)
            .map(|i| aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_1024_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..1024u32)
            .map(|i| {
                let x = (i % 32) as f32;
                let y = (i / 32) as f32;
                aabb_at(Vec3::new(x, y, 0.0), 0.4)
            })
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_eq!(gpu.len(), cpu.nodes.len(), "1024-leaf GPU/CPU node count must match");
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_duplicate_morton_codes() {
        // 4 items at the exact same centre — all share one Morton code.
        // Index tie-break in delta() must keep CPU and GPU in agreement.
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        let aabbs: Vec<Aabb> = (0..4)
            .map(|_| aabb_at(Vec3::ZERO, 0.5))
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }

    #[test]
    fn gpu_lbvh_matches_cpu_random_100_leaves() {
        let Some((device, queue)) = test_device::try_acquire() else { return; };
        // Deterministic pseudo-random AABBs across a 10×10×10 box.
        let mut state: u32 = 0xc0ffee01;
        let mut rand = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) as f32 / 32768.0
        };
        let aabbs: Vec<Aabb> = (0..100)
            .map(|_| {
                let centre = Vec3::new(rand(), rand(), rand()) * 10.0;
                Aabb::from_centre(centre, Vec3::splat(0.2))
            })
            .collect();
        let gpu = run_gpu_lbvh(&device, &queue, &aabbs);
        let items: Vec<(u32, Aabb)> = aabbs.iter().copied().enumerate().map(|(i, a)| (i as u32, a)).collect();
        let cpu = Bvh::build(items);
        assert_gpu_matches_cpu(&gpu, &cpu.nodes);
    }
}
