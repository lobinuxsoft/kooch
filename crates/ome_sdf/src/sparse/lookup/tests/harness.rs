//! Shared probe-pipeline harness for the lookup GPU tests. Encodes
//! `classify → populate → probe-compute` end-to-end against an
//! analytic sphere sampler and reads back results + sparse buffers
//! the per-test assertions need. Lives in `@group(0)` so the
//! lookup-default `@group(2)` does not collide.

use crate::sparse::{
    AnalyticSphereSampler, ClassifyPass, DEFAULT_MARGIN, PopulatePass, ROOT_DIM, SdfSampler,
    SparseGrid, test_device,
};
use glam::Vec3;
use ome_bvh::Aabb;

use super::super::{
    LOOKUP_DEFAULT_GROUP, LOOKUP_DEFAULT_POOL_BINDING, LOOKUP_DEFAULT_ROOT_BINDING,
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
pub(super) const PROBE_HARNESS_WGSL: &str = r#"
struct ProbeUniform {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> probe_positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> probe_results: array<f32>;
@group(0) @binding(2) var<uniform> probe_uniform: ProbeUniform;

@compute @workgroup_size(64)
fn probe_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= probe_uniform.count) {
        return;
    }
    probe_results[gid.x] = sparse_sdf_lookup(probe_positions[gid.x].xyz);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ProbeUniformHost {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

/// Result bundle from one probe run — keeps the readback signature
/// short while exposing the sparse buffers the per-test assertions
/// poke into.
pub(super) struct ProbeRun {
    pub(super) grid: SparseGrid,
    pub(super) results: Vec<f32>,
    pub(super) root_indices: Vec<u32>,
    pub(super) subgrid_pool: Vec<f32>,
}

pub(super) fn run_lookup_probes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    bounds: Aabb,
    max_subgrids: u32,
    probe_positions: &[Vec3],
) -> ProbeRun {
    let grid = SparseGrid::new(device, queue, bounds, max_subgrids);
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
    let lookup_bindings = LookupBindings::new(device);
    lookup_bindings.write(queue, bounds);

    let classify_sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::classify_sampler_bg"),
        layout: classify.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });
    let populate_sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::populate_sampler_bg"),
        layout: populate.sampler_bind_group_layout(),
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
        LOOKUP_DEFAULT_POOL_BINDING,
        LOOKUP_DEFAULT_UNIFORM_BINDING,
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
            LOOKUP_DEFAULT_POOL_BINDING,
            LOOKUP_DEFAULT_UNIFORM_BINDING,
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
        LOOKUP_DEFAULT_POOL_BINDING,
        LOOKUP_DEFAULT_UNIFORM_BINDING,
    );
    let lookup_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::probe::lookup_bg"),
        layout: &lookup_bgl,
        entries: &lookup_bg_entries,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::probe::encoder"),
    });
    classify.record(
        device,
        queue,
        &mut encoder,
        &grid,
        &classify_sampler_bg,
        DEFAULT_MARGIN,
    );
    populate.record(device, queue, &mut encoder, &grid, &populate_sampler_bg);
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
    let root_bytes = test_device::readback(device, queue, grid.root_indices_buffer());
    let root_indices: Vec<u32> = root_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let pool_bytes = test_device::readback(device, queue, grid.subgrid_pool_buffer());
    let subgrid_pool: Vec<f32> = pool_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    ProbeRun {
        grid,
        results,
        root_indices,
        subgrid_pool,
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
