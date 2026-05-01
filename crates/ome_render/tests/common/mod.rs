//! Shared harness for the OmeAccel pool integration tests
//! (`pool_eval_smoke`, `ac1`..`ac7`). Cargo treats `tests/common/mod.rs`
//! as a non-test sibling, so each integration test that pulls it in via
//! `mod common;` recompiles the helpers without spawning extra runners.
//!
//! Owns: device acquisition, the `SmokePrimitive` mirror struct
//! (matches the WGSL `SdfPrimitive` byte layout), the
//! `dispatch_eval_pass` helper that builds the compute pipeline +
//! bind groups + readback, and a few primitive constructors used by
//! every AC test.
//!
//! Anything specific to one AC (sample-point distribution, scene
//! seeds, assertion thresholds) lives in the per-AC file so the
//! intent of each test stays local.

#![allow(dead_code)] // Each test binary touches a different subset.

pub mod gdf;

use bytemuck::{Pod, Zeroable};
use ome_bvh::{IS_RAYMARCH, LeafAabb, OmeAccel, ROLE_RAYMARCH_ADD};
use ome_render::raymarch::POOL_EVAL_SHADER_SOURCE;
use wgpu::util::DeviceExt;

pub const TYPE_SPHERE: u32 = 0;

/// Mirror of the WGSL `SdfPrimitive` — `smoothness` lives in the
/// legacy `_pad0` slot so the byte layout matches.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug)]
pub struct SmokePrimitive {
    pub position: [f32; 3],
    pub type_tag: u32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub smoothness: f32,
    pub params: [f32; 4],
}

impl SmokePrimitive {
    /// Convenience: an axis-aligned identity-rotated sphere of `radius`
    /// at `centre`, role-ADD with no smoothness.
    pub fn sphere(centre: [f32; 3], radius: f32) -> Self {
        Self {
            position: centre,
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [radius, 0.0, 0.0, 0.0],
        }
    }
}

/// 16-byte stride matches the WGSL `array<vec4<f32>>` for the
/// `sample_points` binding.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug)]
pub struct SamplePoint {
    pub p: [f32; 4],
}

impl SamplePoint {
    pub fn at(x: f32, y: f32, z: f32) -> Self {
        Self { p: [x, y, z, 0.0] }
    }
}

/// Build a leaf AABB for a sphere of `radius` at `centre`, role-ADD,
/// flagged `IS_RAYMARCH` so the per-role fold accepts it.
pub fn sphere_leaf(centre: [f32; 3], radius: f32, entity_id: u32) -> LeafAabb {
    LeafAabb {
        aabb_min: [centre[0] - radius, centre[1] - radius, centre[2] - radius],
        flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
        aabb_max: [centre[0] + radius, centre[1] + radius, centre[2] + radius],
        entity_id,
    }
}

/// Lazy adapter acquisition. Returns `None` when no Vulkan / Metal /
/// DX12 backend is available; tests treat that as a skip rather than
/// a failure (same policy the existing GPU tests use).
pub fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ome_render::tests::common"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

/// Build the compute pipeline + bind-group layouts for `cs_eval_smoke`
/// over the OmeAccel pool. Stored together so the per-test entry point
/// recreates pipelines once and dispatches multiple times if needed.
pub struct EvalPipeline {
    pub bgl0: wgpu::BindGroupLayout,
    pub bgl1: wgpu::BindGroupLayout,
    pub pipeline: wgpu::ComputePipeline,
}

impl EvalPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_render::tests::common::module"),
            source: wgpu::ShaderSource::Wgsl(POOL_EVAL_SHADER_SOURCE.into()),
        });
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("common_bgl_io"),
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
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pool_entries: Vec<wgpu::BindGroupLayoutEntry> = (5..=9u32)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .chain(std::iter::once(wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }))
            .collect();
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("common_bgl_pool"),
            entries: &pool_entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("common_pipeline_layout"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("common_pipeline"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_eval_smoke"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { bgl0, bgl1, pipeline }
    }
}

/// Dispatch `cs_eval_smoke` over `samples` against the pool resident
/// in `accel`, read back the `f32` distance per sample, and return
/// the readback vector. Allocates fresh sample / distance buffers per
/// call so the helper is reusable across two consecutive runs of the
/// same scene (used by AC1 + AC6).
pub fn dispatch_eval_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &EvalPipeline,
    accel: &OmeAccel,
    samples: &[SamplePoint],
) -> Vec<f32> {
    let n = samples.len() as u32;
    let sample_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("common_sample_points"),
        contents: bytemuck::cast_slice(samples),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let dist_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("common_sample_distances"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("common_bg_io"),
        layout: &pipeline.bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: sample_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: dist_buf.as_entire_binding() },
        ],
    });
    let bg1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("common_bg_pool"),
        layout: &pipeline.bgl1,
        entries: &[
            wgpu::BindGroupEntry { binding: 5, resource: accel.buffers.tlas_nodes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: accel.buffers.chunk_descriptors.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: accel.buffers.bvh_nodes_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: accel.buffers.leaf_aabbs_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 9, resource: accel.buffers.primitives_pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: accel.buffers.tlas_uniforms.as_entire_binding() },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("common_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("common_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &bg1, &[]);
        pass.dispatch_workgroups((n + 63) / 64, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("common_staging"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc2 = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("common_readback_encoder"),
    });
    enc2.copy_buffer_to_buffer(&dist_buf, 0, &staging, 0, (n as u64) * 4);
    queue.submit(std::iter::once(enc2.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    rx.recv().expect("map_async sender").expect("map_async result");
    let data = slice.get_mapped_range();
    let out: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();
    out
}
