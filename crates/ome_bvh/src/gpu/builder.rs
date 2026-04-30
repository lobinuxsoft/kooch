//! [`BvhGpuBuilder`] — owns the GPU compute pipelines and reusable
//! buffers for the LBVH build (Morton encoding → onesweep radix sort
//! → Karras parallel construction).
//!
//! Single source of truth for state that survives across builds:
//! pipelines (cached via the optional [`wgpu::PipelineCache`] handle),
//! storage buffers (grow-on-demand, never realloc per build), and the
//! timestamp query set (per-pass profiling). The `Bvh::build_gpu` API
//! threads its work through this struct.
//!
//! Designed for **production use day one** (per session 2026-04-26
//! quater feedback): `wgpu::PipelineCache` integration, reusable
//! buffers, async API surface, timestamp query instrumentation.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::gpu::lbvh::{LbvhBuffers, LbvhPipelines};
use crate::gpu::sort::{SortBuffers, SortPipelines};
use crate::gpu::sort_types::ITEMS_PER_TILE;
use crate::gpu::types::{GpuAabb, GpuSceneBounds};

/// Number of timestamp slots: one pair (start/end) per compute pass.
/// Order in the buffer:
///   `[morton_start, morton_end, sort_start, sort_end, build_start, build_end]`.
const TIMESTAMP_QUERY_COUNT: u32 = 6;

/// Per-pass timestamp slot indices. Used by every dispatch helper to
/// write the same query set positions consistently.
const TS_MORTON_START: u32 = 0;
#[allow(dead_code)]
const TS_MORTON_END: u32 = 1;
#[allow(dead_code)]
const TS_SORT_START: u32 = 2;
#[allow(dead_code)]
const TS_SORT_END: u32 = 3;
#[allow(dead_code)]
const TS_BUILD_START: u32 = 4;
#[allow(dead_code)]
const TS_BUILD_END: u32 = 5;

/// Workgroup size for the Morton compute shader. Matches the
/// `@workgroup_size(256)` declaration in `morton.wgsl`. Conservative
/// for portability — RDNA 4 / RTX 4070 happily run 256.
const MORTON_WORKGROUP_SIZE: u32 = 256;

/// Owns the GPU compute infrastructure for the LBVH build pipeline.
///
/// Built once per app instance, reused across every BVH build. Hold
/// it in a long-lived resource (e.g. a `wgpu::Device`-scoped struct
/// in `ome_render` or as a `Resources` entry).
pub struct BvhGpuBuilder {
    pub(crate) morton_pipeline: wgpu::ComputePipeline,
    pub(crate) morton_bgl: wgpu::BindGroupLayout,

    // Reusable buffers — capacities are tracked separately so we only
    // grow (never shrink). All buffers use `STORAGE | COPY_DST`; the
    // staging readback path uses `MAP_READ | COPY_DST`.
    pub(crate) aabbs_buffer: wgpu::Buffer,
    pub(crate) aabbs_capacity: u64,

    pub(crate) scene_bounds_buffer: wgpu::Buffer,

    pub(crate) morton_codes_buffer: wgpu::Buffer,
    pub(crate) morton_codes_capacity: u64,

    /// Onesweep radix sort pipelines + reusable buffers. Driven by
    /// [`Self::dispatch_sort_into`] (pass-through wrapper around
    /// [`crate::gpu::sort::dispatch_sort_into`]).
    pub(crate) sort_pipelines: SortPipelines,
    pub(crate) sort_buffers: SortBuffers,

    /// Karras LBVH constructor pipelines + reusable buffers. Driven by
    /// [`Self::dispatch_lbvh_into`] (pass-through wrapper around
    /// [`crate::gpu::lbvh::dispatch_lbvh_build`]).
    pub(crate) lbvh_pipelines: LbvhPipelines,
    pub(crate) lbvh_buffers: LbvhBuffers,

    /// Timestamp query set + resolve buffer for per-pass profiling.
    /// `None` when the device was not built with
    /// [`wgpu::Features::TIMESTAMP_QUERY`] +
    /// [`wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES`] — the engine
    /// requests these as **optional** features in
    /// [`ome_core::gpu`](../../../../ome_core/gpu/index.html), so the
    /// builder must stay correct on adapters that don't expose them.
    /// Without timestamps the builder skips both `create_query_set` and
    /// the `timestamp_writes` on the morton pass; the validation error
    /// otherwise rejects the entire submission and silently corrupts
    /// downstream `done_buffer` reads (#333).
    pub(crate) timestamps: Option<BvhTimestamps>,
}

/// Per-build timestamp infrastructure. Resolved into `resolve_buffer`
/// which can be mapped + read for per-pass timing.
pub struct BvhTimestamps {
    pub query_set: wgpu::QuerySet,
    pub resolve_buffer: wgpu::Buffer,
    pub readback_buffer: wgpu::Buffer,
    /// `period_ns` reported by the queue. Multiply raw timestamp
    /// deltas by this to get nanoseconds.
    pub period_ns: f32,
}

/// Initial capacity for storage buffers (in items). Grows by
/// `next_power_of_two` when an upload exceeds capacity.
const INITIAL_AABB_CAPACITY: u64 = 256;
const INITIAL_MORTON_CAPACITY: u64 = 256;

impl BvhGpuBuilder {
    /// Build the BVH GPU compute infrastructure. `pipeline_cache` is
    /// optional — pass the engine's shared cache (`ome_core::pipeline_cache`)
    /// to amortise compile time across runs; `None` re-compiles every
    /// pipeline at construction (acceptable for test fixtures).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, pipeline_cache: Option<&wgpu::PipelineCache>) -> Self {
        let morton_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::morton"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/morton.wgsl").into()),
        });

        let morton_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::morton_bgl"),
            entries: &[
                // 0: aabbs (read-only storage)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<GpuAabb>() as u64),
                    },
                    count: None,
                },
                // 1: scene bounds (uniform)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<GpuSceneBounds>() as u64,
                        ),
                    },
                    count: None,
                },
                // 2: morton codes (read-write storage)
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
        });

        let morton_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ome_bvh::morton_pl"),
                bind_group_layouts: &[Some(&morton_bgl)],
                immediate_size: 0,
            });

        let morton_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::morton_pipeline"),
            layout: Some(&morton_pipeline_layout),
            module: &morton_shader,
            entry_point: Some("morton_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        let aabbs_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::aabbs"),
            size: INITIAL_AABB_CAPACITY * std::mem::size_of::<GpuAabb>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bounds_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::scene_bounds"),
            contents: bytemuck::bytes_of(&GpuSceneBounds::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let morton_codes_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::morton_codes"),
            size: INITIAL_MORTON_CAPACITY * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Timestamps are best-effort telemetry; only enable them when
        // the device exposes both feature flags. `ome_core::gpu`
        // requests them as optional features so adapters that support
        // them get per-pass profiling for free; adapters without them
        // silently drop the instrumentation rather than failing the
        // build (#333: validation rejects every dispatch on a device
        // without TIMESTAMP_QUERY, manifesting as a misleading
        // "AABB iteration slack insufficient" panic).
        let supports_timestamps = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            && device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
        let timestamps =
            if supports_timestamps { Some(BvhTimestamps::new(device, queue)) } else { None };

        let sort_pipelines = SortPipelines::new(device, pipeline_cache);
        let sort_buffers = SortBuffers::new(device);
        let lbvh_pipelines = LbvhPipelines::new(device, pipeline_cache);
        let lbvh_buffers = LbvhBuffers::new(device);

        Self {
            morton_pipeline,
            morton_bgl,
            aabbs_buffer,
            aabbs_capacity: INITIAL_AABB_CAPACITY,
            scene_bounds_buffer,
            morton_codes_buffer,
            morton_codes_capacity: INITIAL_MORTON_CAPACITY,
            sort_pipelines,
            sort_buffers,
            lbvh_pipelines,
            lbvh_buffers,
            timestamps,
        }
    }

    /// Reference to the LBVH output nodes buffer. After
    /// [`crate::Bvh::build_gpu`] resolves, this buffer holds the
    /// `2N - 1` [`crate::BvhNode`]s in standard Karras layout.
    ///
    /// **Internal scratch — not stable across builds.** The builder
    /// reuses this buffer in every build, so subsequent calls
    /// overwrite the contents. Consumers that need a stable handle for
    /// rendering should either:
    /// - Hold a [`crate::BvhGpuBuild`] alive (the build's
    ///   [`crate::GpuBvhHandle`] gates access on the build's submission
    ///   index), or
    /// - Copy the contents into their own buffers between builds (PR-4
    ///   raymarch culling does this — see `ome_render::raymarch::bvh`).
    pub fn nodes_buffer(&self) -> &wgpu::Buffer {
        &self.lbvh_buffers.nodes_buffer
    }

    /// Reference to the onesweep-sorted payload-indices buffer. After
    /// [`crate::Bvh::build_gpu`] resolves, this buffer holds `N` u32
    /// entries — `sorted_indices[k]` = original index of the item at
    /// Morton-sorted position `k`.
    ///
    /// **Internal scratch — same caveat as [`Self::nodes_buffer`].**
    pub fn sorted_indices_buffer(&self) -> &wgpu::Buffer {
        &self.sort_buffers.values_a
    }

    /// Grow every owned buffer to fit `count` items. Each sub-component
    /// (morton state, sort, lbvh) tracks its own capacity and only
    /// reallocates if the request exceeds what it already has — buffers
    /// never shrink.
    ///
    /// `pub(crate)` so `gpu::build::Bvh::build_gpu` can call it from
    /// outside this module before recording the encoder.
    pub(crate) fn ensure_capacity(&mut self, device: &wgpu::Device, count: u64) {
        if count > self.aabbs_capacity {
            let new_cap = count.next_power_of_two().max(INITIAL_AABB_CAPACITY);
            self.aabbs_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ome_bvh::aabbs"),
                size: new_cap * std::mem::size_of::<GpuAabb>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.aabbs_capacity = new_cap;
        }
        if count > self.morton_codes_capacity {
            let new_cap = count.next_power_of_two().max(INITIAL_MORTON_CAPACITY);
            self.morton_codes_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ome_bvh::morton_codes"),
                size: new_cap * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            self.morton_codes_capacity = new_cap;
        }
        // Sort needs both keys/values capacity in items and a partition
        // descriptor count derived from `ITEMS_PER_TILE`.
        let partitions = ((count as u32) + ITEMS_PER_TILE - 1) / ITEMS_PER_TILE;
        self.sort_buffers
            .ensure_capacity(device, count, partitions.max(1));
        self.lbvh_buffers.ensure_capacity(device, count);
    }

    /// Upload the AABB list + record the Morton compute pass into the
    /// caller-supplied encoder. Production primitive: `Bvh::build_gpu`
    /// chains this with sort + lbvh on a single encoder before submit.
    ///
    /// Buffer uploads (`queue.write_buffer`) happen before the encoder
    /// commands run within a submission, so it's safe to call this even
    /// when later passes on `encoder` read the uploaded data.
    pub fn dispatch_morton_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        aabbs: &[crate::aabb::Aabb],
    ) {
        let count = aabbs.len() as u64;
        self.ensure_capacity(device, count);

        // Upload AABBs (CPU → GPU). Always uploads exactly `count`
        // items; the buffer may be larger but we only touch the prefix.
        let gpu_aabbs: Vec<GpuAabb> = aabbs.iter().copied().map(GpuAabb::from).collect();
        if !gpu_aabbs.is_empty() {
            queue.write_buffer(
                &self.aabbs_buffer,
                0,
                bytemuck::cast_slice(&gpu_aabbs),
            );
        }
        let scene = GpuSceneBounds::from_aabbs(aabbs);
        queue.write_buffer(&self.scene_bounds_buffer, 0, bytemuck::bytes_of(&scene));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::morton_bg"),
            layout: &self.morton_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.aabbs_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.morton_codes_buffer.as_entire_binding(),
                },
            ],
        });

        let timestamp_writes = self.timestamps.as_ref().map(|ts| {
            wgpu::ComputePassTimestampWrites {
                query_set: &ts.query_set,
                beginning_of_pass_write_index: Some(TS_MORTON_START),
                end_of_pass_write_index: Some(TS_MORTON_END),
            }
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::morton_pass"),
            timestamp_writes,
        });
        pass.set_pipeline(&self.morton_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let workgroups = ((count as u32) + MORTON_WORKGROUP_SIZE - 1) / MORTON_WORKGROUP_SIZE;
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }

    /// Standalone Morton dispatch — creates its own encoder and returns
    /// it for the caller to submit. Used by the unit tests for the
    /// Morton pass in isolation; production code uses
    /// [`Self::dispatch_morton_into`] via `Bvh::build_gpu`.
    pub fn dispatch_morton(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aabbs: &[crate::aabb::Aabb],
    ) -> wgpu::CommandEncoder {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::morton_encoder"),
        });
        self.dispatch_morton_into(device, queue, &mut encoder, aabbs);
        encoder
    }

    // Sort + LBVH dispatches are issued by `Bvh::build_gpu` in
    // `gpu::build` via the free-function primitives in `gpu::sort`
    // and `gpu::lbvh`. We don't expose `&self`-receiving wrappers here
    // because the orchestrator simultaneously borrows disjoint fields
    // of the builder (e.g. `&self.aabbs_buffer` together with
    // `&self.lbvh_pipelines`), and a `&self`-method receiver would
    // conflict with those borrows. The free functions accept the
    // pipelines/buffers as explicit refs, sidestepping that.

    /// Test-only readback of the Morton codes buffer. Returns `count`
    /// codes in a CPU `Vec<u32>`. Production code never roundtrips this
    /// data — it stays GPU-resident through the whole pipeline.
    pub fn readback_morton_for_test(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        count: u64,
    ) -> Vec<u32> {
        let bytes = count * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::morton_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::morton_readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(&self.morton_codes_buffer, 0, &staging, 0, bytes);
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
        receiver.recv().expect("map_async sender dropped").expect("map_async failed");
        let data = slice.get_mapped_range();
        let codes: Vec<u32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        codes
    }
}

impl BvhTimestamps {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("ome_bvh::timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: TIMESTAMP_QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::timestamps_resolve"),
            size: (TIMESTAMP_QUERY_COUNT as u64) * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::timestamps_readback"),
            size: (TIMESTAMP_QUERY_COUNT as u64) * 8,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let period_ns = queue.get_timestamp_period();
        Self {
            query_set,
            resolve_buffer,
            readback_buffer,
            period_ns,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_device {
    /// Acquire a wgpu device + queue for unit tests. Picks any
    /// available adapter (vulkan / metal / dx12 / gl). Returns `None`
    /// when no GPU is available — the test in that case skips itself
    /// rather than failing (CI without a display falls into this path).
    ///
    /// `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_PASSES` features
    /// are requested unconditionally — the BVH builder writes timestamps
    /// in every dispatch and would fail to validate a pipeline without
    /// them. Adapters that don't expose the feature are skipped.
    pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;

            let supports_ts =
                adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
            let supports_ts_inside =
                adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
            if !supports_ts || !supports_ts_inside {
                return None;
            }

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("ome_bvh::test_device"),
                    required_features: wgpu::Features::TIMESTAMP_QUERY
                        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES,
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .ok()?;
            Some((device, queue))
        })
    }
}
