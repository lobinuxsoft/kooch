//! Shared probe-pipeline harness for the lookup GPU tests. Encodes
//! the full LOD cascade (`chunk_lod → classify[0..3] →
//! populate_finalize[0..3] → populate[0..3] → downsample[0..2]`) plus
//! a probe compute that calls `sparse_sdf_lookup` once per thread,
//! reads back results + canonical root_indices the per-test
//! assertions need. Lives in `@group(0)` so the lookup-default
//! `@group(2)` does not collide.

use crate::voxel::{
    AnalyticSphereSampler, CASCADE_COUNT, ClassifyPass, DEFAULT_MARGIN, DownsamplePass,
    LOD_COUNT, PopulatePass, ROOT_DIM, SdfSampler, SparseGrid, test_device,
};
use glam::Vec3;
use ome_bvh::Aabb;

use super::super::{
    LOOKUP_DEFAULT_GROUP, LOOKUP_DEFAULT_MASK_BINDING, LOOKUP_DEFAULT_POOL_BINDINGS,
    LOOKUP_DEFAULT_ROOT_BINDING, LOOKUP_DEFAULT_SAMPLER_BINDING,
    LOOKUP_DEFAULT_UNIFORM_BINDING, LookupBindings, lookup_wgsl,
};

pub(super) const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
pub(super) const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

pub(super) fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

/// Probe pipeline harness — splices `lookup_wgsl(default layout)`
/// ahead of a tiny compute that calls `sparse_sdf_lookup` once per
/// thread, writing into a results buffer the host reads back.
///
/// The harness pins `target_voxel_size` to `cell_size_base` (LOD 0
/// pitch) by default — most tests want max-detail lookups.
pub(super) const PROBE_HARNESS_WGSL: &str = r#"
struct ProbeUniform {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    target_voxel_size_xyz: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> probe_positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> probe_results: array<f32>;
@group(0) @binding(2) var<uniform> probe_uniform: ProbeUniform;

@compute @workgroup_size(64)
fn probe_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= probe_uniform.count) {
        return;
    }
    let voxel_size = probe_uniform.target_voxel_size_xyz.x;
    probe_results[gid.x] = sparse_sdf_lookup(probe_positions[gid.x].xyz, voxel_size);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeUniformHost {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    target_voxel_size_xyz: [f32; 4],
}

/// Result bundle from one probe run.
pub(super) struct ProbeRun {
    pub(super) grid: SparseGrid,
    pub(super) results: Vec<f32>,
    /// LOD 0 root_indices — the canonical buffer the lookup binds
    /// (post-cascade all LODs hold the same value).
    pub(super) root_indices: Vec<u32>,
}

/// Run the full cascade + probe with a default target_voxel_size of
/// `cell_size / SUBGRID_DIM` (LOD 0 voxel pitch — max detail).
pub(super) fn run_lookup_probes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    bounds: Aabb,
    max_subgrids: u32,
    probe_positions: &[Vec3],
) -> ProbeRun {
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let voxel_pitch = cell_size.x.min(cell_size.y).min(cell_size.z) / 16.0;
    run_lookup_probes_with_target(
        device, queue, sampler, bounds, max_subgrids, probe_positions, voxel_pitch,
    )
}

pub(super) fn run_lookup_probes_with_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    bounds: Aabb,
    max_subgrids: u32,
    probe_positions: &[Vec3],
    target_voxel_size: f32,
) -> ProbeRun {
    let grid = SparseGrid::new(device, queue, bounds, max_subgrids);
    // Force every LOD bit on so the per-LOD classify pipelines all
    // run, populate fills every atlas at native resolution, and the
    // lookup's mask-clamp logic can return any LOD the test asks for.
    let all = (1u32 << LOD_COUNT) - 1;
    queue.write_buffer(grid.chunk_lod_mask_buffer(), 0, &all.to_le_bytes());

    let classify = ClassifyPass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let populate = PopulatePass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let downsample = DownsamplePass::new(device);
    let lookup_bindings = LookupBindings::new(device);
    lookup_bindings.write(queue, bounds);

    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::sampler_bg"),
        layout: classify.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    // Probe positions buffer — vec4 padded so std140 alignment holds.
    let positions_padded: Vec<[f32; 4]> = probe_positions
        .iter()
        .map(|p| [p.x, p.y, p.z, 0.0])
        .collect();
    let positions_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::probe::positions"),
        size: (positions_padded.len() * std::mem::size_of::<[f32; 4]>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &positions_buffer,
        0,
        bytemuck::cast_slice(&positions_padded),
    );

    let results_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::probe::results"),
        size: (probe_positions.len() * std::mem::size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &results_buffer,
        0,
        &vec![0u8; probe_positions.len() * std::mem::size_of::<f32>()],
    );

    let probe_uniform = ProbeUniformHost {
        count: probe_positions.len() as u32,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
        target_voxel_size_xyz: [target_voxel_size, 0.0, 0.0, 0.0],
    };
    let probe_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::probe::uniform"),
        size: std::mem::size_of::<ProbeUniformHost>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &probe_uniform_buffer,
        0,
        bytemuck::bytes_of(&probe_uniform),
    );

    // Bind group 0 — probe-local resources.
    let probe_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test::probe::bgl"),
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

    // Bind group 2 — lookup globals (default layout).
    let lookup_layout_entries = LookupBindings::layout_entries(
        LOOKUP_DEFAULT_ROOT_BINDING,
        LOOKUP_DEFAULT_POOL_BINDINGS,
        LOOKUP_DEFAULT_UNIFORM_BINDING,
        LOOKUP_DEFAULT_SAMPLER_BINDING,
        LOOKUP_DEFAULT_MASK_BINDING,
    );
    let lookup_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test::probe::lookup_bgl"),
        entries: &lookup_layout_entries,
    });

    let probe_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("test::probe::pipeline_layout"),
        // Group 1 unused — wgpu requires None placeholders so group
        // indices line up with the WGSL `@group` attributes.
        bind_group_layouts: &[Some(&probe_bgl), None, Some(&lookup_bgl)],
        immediate_size: 0,
    });

    let combined_src = format!(
        "{}{}",
        lookup_wgsl(
            LOOKUP_DEFAULT_GROUP,
            LOOKUP_DEFAULT_ROOT_BINDING,
            LOOKUP_DEFAULT_POOL_BINDINGS,
            LOOKUP_DEFAULT_UNIFORM_BINDING,
            LOOKUP_DEFAULT_SAMPLER_BINDING,
            LOOKUP_DEFAULT_MASK_BINDING,
        ),
        PROBE_HARNESS_WGSL,
    );
    let probe_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test::probe::module"),
        source: wgpu::ShaderSource::Wgsl(combined_src.into()),
    });
    let probe_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test::probe::pipeline"),
        layout: Some(&probe_pipeline_layout),
        module: &probe_module,
        entry_point: Some("probe_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let probe_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::bg"),
        layout: &probe_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: positions_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: results_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: probe_uniform_buffer.as_entire_binding(),
            },
        ],
    });
    let lookup_bg_entries = lookup_bindings.bind_group_entries(
        &grid,
        LOOKUP_DEFAULT_ROOT_BINDING,
        LOOKUP_DEFAULT_POOL_BINDINGS,
        LOOKUP_DEFAULT_UNIFORM_BINDING,
        LOOKUP_DEFAULT_SAMPLER_BINDING,
        LOOKUP_DEFAULT_MASK_BINDING,
    );
    let lookup_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::lookup_bg"),
        layout: &lookup_bgl,
        entries: &lookup_bg_entries,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::probe::encoder"),
    });

    // Skip chunk_lod — the synthetic all-ones mask written above gives
    // every LOD activity in the lookup. The cascade producer runs at
    // LOD 0 only (`base_lod = 0` invariant); the downsample chain
    // fills LODs 1..3 via the box-filter cascade.
    classify.record(
        device, queue, &mut encoder, &grid, &sampler_bg, 0, DEFAULT_MARGIN,
    );
    populate.record_finalize(device, &mut encoder, &grid, 0);
    populate.record_populate(
        device, queue, &mut encoder, &grid, &sampler_bg, 0,
    );
    for cascade_idx in 0..(CASCADE_COUNT as u32) {
        downsample.record_cascade(device, &mut encoder, &grid, cascade_idx);
    }

    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("test::probe::pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&probe_pipeline);
        pass.set_bind_group(0, &probe_bg, &[]);
        pass.set_bind_group(2, &lookup_bg, &[]);
        let workgroups = (probe_positions.len() as u32).div_ceil(64);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let result_bytes = test_device::readback(device, queue, &results_buffer);
    let results: Vec<f32> = result_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let root_bytes = test_device::readback(device, queue, grid.root_indices_buffer(0));
    let root_indices: Vec<u32> = root_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    ProbeRun {
        grid,
        results,
        root_indices,
    }
}

/// Decompose a populated cell index into its world-space corner.
pub(super) fn cell_min_world(cell_idx: u32, bounds: Aabb) -> Vec3 {
    let cz = cell_idx / (ROOT_DIM * ROOT_DIM);
    let cy = (cell_idx / ROOT_DIM) % ROOT_DIM;
    let cx = cell_idx % ROOT_DIM;
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    bounds.min + Vec3::new(cx as f32, cy as f32, cz as f32) * cell_size
}
