//! Onesweep radix sort dispatch + pipelines.
//!
//! Hosts the WGSL pipelines for `init` (clear scratch buffers),
//! `histogram` (count digits across 4 passes in one dispatch),
//! `exclusive_scan` (prefix-sum the histogram per pass) and (in
//! round C) `scatter` (decoupled-lookback chained scan + scatter).
//!
//! [`SortPipelines`] owns the compiled pipelines and their bind group
//! layouts (created once, reused across builds). [`SortBuffers`] owns
//! the storage buffers (grow-on-demand, reused across builds).
//!
//! End-to-end sort orchestration lives in `Bvh::build_gpu` (subtask 4
//! of PR-3); this module exposes the per-pass primitives so each can
//! be unit-tested in isolation against the CPU reference.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::gpu::sort_types::{
    OnesweepConfig, RADIX_BUCKETS, RADIX_PASSES, SORT_WORKGROUP_SIZE,
    global_histogram_size_bytes, partition_descriptors_size_bytes,
};

/// Initial capacity for the keys / values buffers, in items.
/// Grows by `next_power_of_two` on demand.
const INITIAL_KEYS_CAPACITY: u64 = 1024;

/// Initial capacity for the partition descriptors, in partitions.
const INITIAL_PARTITIONS: u32 = 64;

/// Owned compute pipelines for the sort. Built once per `BvhGpuBuilder`,
/// reused across builds. `pipeline_cache` integration is wired through
/// the constructor.
pub struct SortPipelines {
    pub init_pipeline: wgpu::ComputePipeline,
    pub init_bgl: wgpu::BindGroupLayout,
    pub histogram_pipeline: wgpu::ComputePipeline,
    pub histogram_bgl: wgpu::BindGroupLayout,
    pub scan_pipeline: wgpu::ComputePipeline,
    pub scan_bgl: wgpu::BindGroupLayout,
    pub scatter_pipeline: wgpu::ComputePipeline,
    pub scatter_bgl: wgpu::BindGroupLayout,
}

/// Reusable GPU buffers for the sort. Grow on demand, never realloc'd
/// per build (per the production-from-day-1 rule). The keys / values
/// buffers are ping-pong: each radix pass swaps `*_in` and `*_out`.
pub struct SortBuffers {
    pub global_histogram: wgpu::Buffer,
    pub partition_descriptors: wgpu::Buffer,
    pub partitions_capacity: u32,
    pub keys_a: wgpu::Buffer,
    pub keys_b: wgpu::Buffer,
    pub values_a: wgpu::Buffer,
    pub values_b: wgpu::Buffer,
    pub keys_capacity: u64,
    pub config_buffer: wgpu::Buffer,
    pub init_config_buffer: wgpu::Buffer,
    /// One config buffer per pass — `queue.write_buffer` against the
    /// same buffer between dispatches doesn't serialise within a single
    /// submission, so each pass needs its own buffer (or dynamic-offset
    /// uniforms; simplicity wins at 4 × 16 bytes).
    pub scan_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize],
    /// Same per-pass story for the scatter config — each pass needs
    /// its own `pass_shift` written before its dispatch.
    pub scatter_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct InitConfig {
    pub histogram_count: u32,
    pub descriptor_count: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct HistogramConfig {
    pub count: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub struct ScanConfig {
    pub pass_index: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl SortPipelines {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        // -- init pipeline --
        let init_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_init"),
            source: wgpu::ShaderSource::Wgsl(super::ONESWEEP_INIT_WGSL.into()),
        });
        let init_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_init_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
        let init_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_init_pl"),
            bind_group_layouts: &[Some(&init_bgl)],
            immediate_size: 0,
        });
        let init_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::sort_init_pipeline"),
            layout: Some(&init_pl),
            module: &init_shader,
            entry_point: Some("init_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- histogram pipeline --
        let histogram_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_histogram"),
            source: wgpu::ShaderSource::Wgsl(super::ONESWEEP_HISTOGRAM_WGSL.into()),
        });
        let histogram_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_histogram_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
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
        let histogram_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_histogram_pl"),
            bind_group_layouts: &[Some(&histogram_bgl)],
            immediate_size: 0,
        });
        let histogram_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ome_bvh::sort_histogram_pipeline"),
                layout: Some(&histogram_pl),
                module: &histogram_shader,
                entry_point: Some("count_main"),
                compilation_options: Default::default(),
                cache: pipeline_cache,
            });

        // -- scan pipeline --
        let scan_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_scan"),
            source: wgpu::ShaderSource::Wgsl(super::ONESWEEP_EXCLUSIVE_SCAN_WGSL.into()),
        });
        let scan_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_scan_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
        let scan_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_scan_pl"),
            bind_group_layouts: &[Some(&scan_bgl)],
            immediate_size: 0,
        });
        let scan_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::sort_scan_pipeline"),
            layout: Some(&scan_pl),
            module: &scan_shader,
            entry_point: Some("scan_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        // -- scatter pipeline --
        let scatter_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::sort_scatter"),
            source: wgpu::ShaderSource::Wgsl(super::ONESWEEP_SCATTER_WGSL.into()),
        });
        // 5 storage + 1 storage atomic + 1 uniform = 7 entries.
        let scatter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::sort_scatter_bgl"),
            entries: &[
                // 0: keys_in
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                // 1: values_in
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
                // 2: keys_out
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
                // 3: values_out
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
                // 4: global_histogram (read-only — already prefix-summed)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                // 5: partition_descriptors (atomic read/write)
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
                // 6: scatter cfg uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
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
        let scatter_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::sort_scatter_pl"),
            bind_group_layouts: &[Some(&scatter_bgl)],
            immediate_size: 0,
        });
        let scatter_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ome_bvh::sort_scatter_pipeline"),
                layout: Some(&scatter_pl),
                module: &scatter_shader,
                entry_point: Some("scatter_main"),
                compilation_options: Default::default(),
                cache: pipeline_cache,
            });

        Self {
            init_pipeline,
            init_bgl,
            histogram_pipeline,
            histogram_bgl,
            scan_pipeline,
            scan_bgl,
            scatter_pipeline,
            scatter_bgl,
        }
    }
}

impl SortBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let global_histogram = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_global_histogram"),
            size: global_histogram_size_bytes(),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let partition_descriptors = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_partition_descriptors"),
            size: partition_descriptors_size_bytes(INITIAL_PARTITIONS),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let keys_a = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "keys_a");
        let keys_b = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "keys_b");
        let values_a = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "values_a");
        let values_b = make_keys_buffer(device, INITIAL_KEYS_CAPACITY, "values_b");
        let config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::sort_config"),
            contents: bytemuck::bytes_of(&OnesweepConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let init_config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::sort_init_config"),
            contents: bytemuck::bytes_of(&InitConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scan_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize] = std::array::from_fn(|i| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("ome_bvh::sort_scan_config_{i}")),
                contents: bytemuck::bytes_of(&ScanConfig {
                    pass_index: i as u32,
                    _pad0: 0,
                    _pad1: 0,
                    _pad2: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        // Scatter configs filled at dispatch time with the live count
        // and partition_count; only `pass_shift` is fixed per pass.
        let scatter_config_buffers: [wgpu::Buffer; RADIX_PASSES as usize] =
            std::array::from_fn(|i| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("ome_bvh::sort_scatter_config_{i}")),
                    contents: bytemuck::bytes_of(&OnesweepConfig::new(0, i as u32)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            });
        Self {
            global_histogram,
            partition_descriptors,
            partitions_capacity: INITIAL_PARTITIONS,
            keys_a,
            keys_b,
            values_a,
            values_b,
            keys_capacity: INITIAL_KEYS_CAPACITY,
            config_buffer,
            init_config_buffer,
            scan_config_buffers,
            scatter_config_buffers,
        }
    }

    /// Grow the keys/values buffers to fit `count` items, and the
    /// partition descriptors buffer to fit `partition_count` partitions.
    pub fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        count: u64,
        partition_count: u32,
    ) {
        if count > self.keys_capacity {
            let new_cap = count.next_power_of_two().max(INITIAL_KEYS_CAPACITY);
            self.keys_a = make_keys_buffer(device, new_cap, "keys_a");
            self.keys_b = make_keys_buffer(device, new_cap, "keys_b");
            self.values_a = make_keys_buffer(device, new_cap, "values_a");
            self.values_b = make_keys_buffer(device, new_cap, "values_b");
            self.keys_capacity = new_cap;
        }
        if partition_count > self.partitions_capacity {
            let new_cap = partition_count.next_power_of_two().max(INITIAL_PARTITIONS);
            self.partition_descriptors = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ome_bvh::sort_partition_descriptors"),
                size: partition_descriptors_size_bytes(new_cap),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.partitions_capacity = new_cap;
        }
    }
}

fn make_keys_buffer(device: &wgpu::Device, capacity: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("ome_bvh::sort_{label}")),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Dispatch helper: clears the global histogram + partition descriptor
/// section for one pass. Caller is responsible for ensuring buffers
/// exist (call `SortBuffers::ensure_capacity` first).
pub fn dispatch_init(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    partition_count: u32,
) {
    let cfg = InitConfig {
        histogram_count: RADIX_PASSES * RADIX_BUCKETS,
        descriptor_count: partition_count * RADIX_BUCKETS,
        _pad0: 0,
        _pad1: 0,
    };
    queue.write_buffer(&buffers.init_config_buffer, 0, bytemuck::bytes_of(&cfg));

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_init_bg"),
        layout: &pipelines.init_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.partition_descriptors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.init_config_buffer.as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_init_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.init_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    // 1 workgroup is enough for the small histogram; descriptor cleanup
    // uses the strided loop in the shader.
    pass.dispatch_workgroups(1, 1, 1);
}

/// Dispatch helper: count digits across all 4 passes in one dispatch.
/// `keys_buffer` is the input array of u32 (Morton codes); the output
/// is `buffers.global_histogram`.
pub fn dispatch_histogram(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    keys_buffer: &wgpu::Buffer,
    count: u32,
) {
    let cfg = HistogramConfig {
        count,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
    };
    queue.write_buffer(&buffers.config_buffer, 0, bytemuck::bytes_of(&cfg));

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_histogram_bg"),
        layout: &pipelines.histogram_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: keys_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_histogram_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.histogram_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
        / crate::gpu::sort_types::ITEMS_PER_TILE;
    pass.dispatch_workgroups(partitions.max(1), 1, 1);
}

/// Dispatch helper: convert one pass's bucket counts into exclusive
/// prefix sums (per-bucket starting offsets in the sorted output).
/// One dispatch per pass; uses the per-pass static config buffer
/// initialised at `SortBuffers` construction.
pub fn dispatch_exclusive_scan(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    pass_index: u32,
) {
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_scan_bg"),
        layout: &pipelines.scan_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.scan_config_buffers[pass_index as usize].as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_scan_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.scan_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    // 1 workgroup × 256 threads handles 256 buckets.
    pass.dispatch_workgroups(1, 1, 1);
    let _ = SORT_WORKGROUP_SIZE; // keep referenced for future scatter use
}

/// Dispatch helper: per-pass scatter via decoupled-lookback chained
/// scan + scatter to output. The caller must clear partition
/// descriptors via `dispatch_init` BEFORE this scatter dispatch (the
/// onesweep state machine assumes every descriptor starts INVALID).
///
/// `pass_index` selects: which radix digit (`pass_shift = pass_index *
/// 8`), which scatter config buffer to use, and which `keys_in/out` +
/// `values_in/out` buffer pair (ping-pong: even passes read `_a`,
/// write `_b`; odd passes swap).
pub fn dispatch_scatter(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
    pass_index: u32,
) {
    let cfg = OnesweepConfig::new(count, pass_index);
    queue.write_buffer(
        &buffers.scatter_config_buffers[pass_index as usize],
        0,
        bytemuck::bytes_of(&cfg),
    );

    let (keys_in, keys_out, values_in, values_out) = if pass_index % 2 == 0 {
        (&buffers.keys_a, &buffers.keys_b, &buffers.values_a, &buffers.values_b)
    } else {
        (&buffers.keys_b, &buffers.keys_a, &buffers.values_b, &buffers.values_a)
    };

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::sort_scatter_bg"),
        layout: &pipelines.scatter_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: keys_in.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: values_in.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: keys_out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: values_out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.global_histogram.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers.partition_descriptors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: buffers.scatter_config_buffers[pass_index as usize]
                    .as_entire_binding(),
            },
        ],
    });

    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::sort_scatter_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.scatter_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(cfg.partition_count.max(1), 1, 1);
}

/// Full sort orchestration recorded into the caller-supplied encoder:
/// init → histogram (all passes) → exclusive scan (4 dispatches) →
/// scatter (4 dispatches with ping-pong).
///
/// Caller is responsible for placing the input keys in `buffers.keys_a`
/// (and matching `values_a` if needed) before submitting the encoder.
/// On submission completion, sorted keys land in `buffers.keys_a` and
/// the permuted values in `buffers.values_a` (even pass count → result
/// returns to the `_a` slot).
///
/// **Important**: the scan over the global histogram is destructive
/// (in-place: counts → exclusive prefix). Each scatter pass needs the
/// scanned histogram for its specific pass index; we run all 4 scans
/// in one batch BEFORE any scatter, then scatter consumes them in
/// order. The descriptor table is cleared by `dispatch_init` at the
/// start; subsequent passes need re-clearing because each scatter
/// publishes new descriptors. We re-issue clear-only between passes.
pub fn dispatch_sort_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
) {
    let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
        / crate::gpu::sort_types::ITEMS_PER_TILE;

    // 1. Clear histogram + descriptors.
    dispatch_init(device, queue, encoder, pipelines, buffers, partitions);

    // 2. Compute per-pass histograms (all 4 in one dispatch).
    dispatch_histogram(
        device,
        queue,
        encoder,
        pipelines,
        buffers,
        &buffers.keys_a,
        count,
    );

    // 3. Exclusive scan each pass's histogram in place.
    for pass_index in 0..RADIX_PASSES {
        dispatch_exclusive_scan(device, encoder, pipelines, buffers, pass_index);
    }

    // 4. Scatter each pass with ping-pong. Re-clear descriptors before
    // each pass (the onesweep state machine starts from all-INVALID).
    // CRITICAL: clear ONLY descriptors, NOT the global histogram —
    // the histogram holds the prefix-summed offsets that every scatter
    // pass reads. `encoder.clear_buffer` is encoder-ordered (unlike
    // `queue.write_buffer`) so the clear lands before the dispatch.
    for pass_index in 0..RADIX_PASSES {
        if pass_index > 0 {
            encoder.clear_buffer(&buffers.partition_descriptors, 0, None);
        }
        dispatch_scatter(device, queue, encoder, pipelines, buffers, count, pass_index);
    }
}

/// Standalone sort orchestration — creates its own encoder and returns
/// it for the caller to submit. Wrapper around [`dispatch_sort_into`]
/// used by the existing onesweep unit tests; production code records
/// the sort into a shared encoder via `dispatch_sort_into` so morton +
/// sort + lbvh share a single submission.
pub fn dispatch_sort(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &SortPipelines,
    buffers: &SortBuffers,
    count: u32,
) -> wgpu::CommandEncoder {
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::sort_encoder"),
        });
    dispatch_sort_into(device, queue, &mut encoder, pipelines, buffers, count);
    encoder
}

/// Test-only readback of the global histogram. Returns
/// `[pass][bucket]` as a flat `Vec<u32>` of `4 * 256 = 1024` entries.
#[cfg(test)]
pub fn readback_histogram_for_test(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffers: &SortBuffers,
) -> Vec<u32> {
    let bytes = global_histogram_size_bytes();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::sort_histogram_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::sort_histogram_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(&buffers.global_histogram, 0, &staging, 0, bytes);
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
    let v = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::builder::test_device;
    use crate::gpu::sort_types::RADIX_BITS;

    fn cpu_histogram(keys: &[u32]) -> Vec<u32> {
        let mut hist = vec![0u32; (RADIX_PASSES * RADIX_BUCKETS) as usize];
        for &k in keys {
            for p in 0..RADIX_PASSES {
                let digit = ((k >> (p * RADIX_BITS)) & 0xFF) as usize;
                hist[(p * RADIX_BUCKETS) as usize + digit] += 1;
            }
        }
        hist
    }

    fn cpu_exclusive_scan(hist: &[u32]) -> Vec<u32> {
        // Per-pass exclusive prefix sum.
        let mut out = hist.to_vec();
        for p in 0..RADIX_PASSES as usize {
            let base = p * RADIX_BUCKETS as usize;
            let slice = &mut out[base..base + RADIX_BUCKETS as usize];
            let mut acc = 0u32;
            for v in slice.iter_mut() {
                let n = *v;
                *v = acc;
                acc += n;
            }
        }
        out
    }

    fn upload_keys(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffers: &mut SortBuffers,
        keys: &[u32],
    ) -> u32 {
        let count = keys.len() as u32;
        let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
            / crate::gpu::sort_types::ITEMS_PER_TILE;
        buffers.ensure_capacity(device, count as u64, partitions);
        if !keys.is_empty() {
            queue.write_buffer(&buffers.keys_a, 0, bytemuck::cast_slice(keys));
        }
        partitions
    }

    #[test]
    fn histogram_matches_cpu_random_keys() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        // 5000 deterministic-pseudo-random u32 keys.
        let mut state = 0xfeedfeedu32;
        let keys: Vec<u32> = (0..5000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();

        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        let cpu = cpu_histogram(&keys);
        assert_eq!(gpu, cpu, "GPU histogram must match CPU reference");
    }

    #[test]
    fn histogram_total_is_count_per_pass() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let keys: Vec<u32> = (0..100u32).collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        // Each pass's 256 buckets must sum to `keys.len()`.
        for p in 0..RADIX_PASSES as usize {
            let base = p * RADIX_BUCKETS as usize;
            let total: u32 = gpu[base..base + RADIX_BUCKETS as usize].iter().sum();
            assert_eq!(total, keys.len() as u32, "pass {p} total mismatch");
        }
    }

    #[test]
    fn exclusive_scan_matches_cpu() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut state = 0xcafebabeu32;
        let keys: Vec<u32> = (0..2000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        for pass in 0..RADIX_PASSES {
            dispatch_exclusive_scan(&device, &mut encoder, &pipelines, &buffers, pass);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        let cpu_hist = cpu_histogram(&keys);
        let cpu_scan = cpu_exclusive_scan(&cpu_hist);
        assert_eq!(gpu, cpu_scan, "exclusive scan must match CPU reference");
    }

    /// Test-only readback of the keys array (after sort lands here).
    fn readback_keys(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        keys_buffer: &wgpu::Buffer,
        count: u32,
    ) -> Vec<u32> {
        let bytes = (count as u64) * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_keys_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::sort_keys_readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(keys_buffer, 0, &staging, 0, bytes);
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
        let v = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
        drop(data);
        staging.unmap();
        v
    }

    fn upload_keys_and_values(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffers: &mut SortBuffers,
        keys: &[u32],
    ) -> u32 {
        let count = keys.len() as u32;
        let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
            / crate::gpu::sort_types::ITEMS_PER_TILE;
        buffers.ensure_capacity(device, count as u64, partitions);
        if !keys.is_empty() {
            queue.write_buffer(&buffers.keys_a, 0, bytemuck::cast_slice(keys));
            // Values = original index 0..count.
            let values: Vec<u32> = (0..count).collect();
            queue.write_buffer(&buffers.values_a, 0, bytemuck::cast_slice(&values));
        }
        partitions
    }

    #[test]
    fn full_sort_matches_cpu_sort_small() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut keys: Vec<u32> = vec![3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5];
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);

        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys);
    }

    #[test]
    fn full_sort_matches_cpu_sort_random() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut state = 0xfeedfeedu32;
        let mut keys: Vec<u32> = (0..2000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);
        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys, "GPU sort must match CPU sort byte-for-byte");
    }

    #[test]
    fn full_sort_handles_multi_partition() {
        // Larger than ITEMS_PER_TILE — exercises the decoupled-lookback
        // chained scan across partitions.
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        // 10 000 keys = 4 partitions (ITEMS_PER_TILE = 3072).
        let mut state = 0xcafebabeu32;
        let mut keys: Vec<u32> = (0..10_000)
            .map(|_| {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                state
            })
            .collect();
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);
        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys);
    }

    #[test]
    fn exclusive_scan_first_bucket_is_zero() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let keys: Vec<u32> = (0..50u32).collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        for pass in 0..RADIX_PASSES {
            dispatch_exclusive_scan(&device, &mut encoder, &pipelines, &buffers, pass);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        for p in 0..RADIX_PASSES as usize {
            assert_eq!(
                gpu[p * RADIX_BUCKETS as usize],
                0,
                "pass {p} bucket 0 exclusive scan must be 0"
            );
        }
    }
}
