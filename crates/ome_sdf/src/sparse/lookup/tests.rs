//! Tests for [`crate::sparse::lookup`] — kept in their own file so
//! the impl module stays under the no-monolithic threshold. The GPU
//! tests share a probe-pipeline harness ([`run_lookup_probes`]) that
//! runs `classify → populate → probe-compute` end-to-end against an
//! analytic sphere sampler so each test only encodes its own
//! assertion.
//!
//! Every GPU test gates on [`test_device::try_acquire`] and skips when
//! no adapter is available — same convention as the other sparse
//! modules.

use super::{
    LOOKUP_BODY_WGSL, LOOKUP_DEFAULT_GROUP, LOOKUP_DEFAULT_POOL_BINDING,
    LOOKUP_DEFAULT_ROOT_BINDING, LOOKUP_DEFAULT_UNIFORM_BINDING, LookupBindings, lookup_wgsl,
};
use crate::sparse::{
    ALLOC_FAILED_SENTINEL, AnalyticSphereSampler, ClassifyPass, DEFAULT_MARGIN, PopulatePass,
    ROOT_CELLS, ROOT_DIM, SUBGRID_DIM, SUBGRID_VOXELS, SdfSampler, SparseGrid, test_device,
};
use glam::Vec3;
use ome_bvh::Aabb;

const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

/// Probe pipeline harness — splices `lookup_wgsl(default layout)`
/// ahead of a tiny compute that calls `sparse_sdf_lookup` once per
/// thread, writing into a results buffer the host reads back. Lives
/// in `@group(0)` so the lookup-default `@group(2)` does not collide.
const PROBE_HARNESS_WGSL: &str = r#"
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
struct ProbeRun {
    grid: SparseGrid,
    results: Vec<f32>,
    root_indices: Vec<u32>,
    subgrid_pool: Vec<f32>,
}

fn run_lookup_probes(
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

#[test]
fn lookup_body_with_default_layout_parses_and_validates() {
    let combined = format!(
        "{}{}",
        lookup_wgsl(
            LOOKUP_DEFAULT_GROUP,
            LOOKUP_DEFAULT_ROOT_BINDING,
            LOOKUP_DEFAULT_POOL_BINDING,
            LOOKUP_DEFAULT_UNIFORM_BINDING,
        ),
        PROBE_HARNESS_WGSL,
    );
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("default lookup layout should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("default lookup layout should validate");
}

#[test]
fn lookup_body_with_alternative_layout_validates() {
    // Raymarcher-style override — single bind group with the lookup
    // globals slotted in 5/6/7 alongside other resources. The probe
    // harness still binds in `@group(0)`; assemble a stand-alone shim
    // that exercises only `lookup_wgsl(0, 5, 6, 7)` plus a no-op
    // entry point so naga walks the `sparse_sdf_lookup` body once.
    let shim = r#"
@compute @workgroup_size(1)
fn shim_main() {
    _ = sparse_sdf_lookup(vec3<f32>(0.0, 0.0, 0.0));
}
"#;
    let combined = format!("{}{}", lookup_wgsl(0, 5, 6, 7), shim);
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("alternative lookup layout should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("alternative lookup layout should validate");
}

#[test]
fn lookup_wgsl_constants_match_host() {
    // Defensive grep — catches drift between the WGSL local constants
    // and the host-side mirrors. Same approach the other sparse tests
    // use.
    assert!(
        LOOKUP_BODY_WGSL.contains(&format!("LOOKUP_ROOT_DIM: u32 = {ROOT_DIM}u")),
    );
    assert!(
        LOOKUP_BODY_WGSL
            .contains(&format!("LOOKUP_SUBGRID_DIM: u32 = {SUBGRID_DIM}u")),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains(&format!(
            "LOOKUP_SUBGRID_VOXELS: u32 = {SUBGRID_VOXELS}u",
        )),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains("LOOKUP_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu"),
    );
    assert!(
        LOOKUP_BODY_WGSL.contains("LOOKUP_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu"),
    );
}

/// Decompose a populated cell index into its world-space corner.
fn cell_min_world(cell_idx: u32, bounds: Aabb) -> Vec3 {
    let cz = cell_idx / (ROOT_DIM * ROOT_DIM);
    let cy = (cell_idx / ROOT_DIM) % ROOT_DIM;
    let cx = cell_idx % ROOT_DIM;
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    bounds.min + Vec3::new(cx as f32, cy as f32, cz as f32) * cell_size
}

#[test]
fn lookup_at_voxel_corners_returns_pool_values() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_at_voxel_corners_returns_pool_values: no GPU");
        return;
    };
    // First run: no probes — used only to discover which cells got
    // allocated, then we craft the corner probes.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let bounds = test_bounds();
    // Probe at one position (origin of bounds); we will read
    // root_indices to pick allocated cells, then re-run with the
    // crafted positions.
    let bootstrap = run_lookup_probes(
        &device, &queue, &sampler, bounds, 1024, &[Vec3::splat(0.0)],
    );

    // Pick up to 10 allocated cells deterministically (lowest cell
    // indices first → reproducible across runs).
    let allocated_cells: Vec<u32> = (0..ROOT_CELLS)
        .filter(|&idx| bootstrap.root_indices[idx as usize] < 1024)
        .take(10)
        .collect();
    assert!(
        !allocated_cells.is_empty(),
        "test scene mis-tuned: expected at least one allocated cell",
    );

    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    // Deterministic per-cell voxel sampling — 50 voxels chosen via a
    // cheap LCG over (cell_idx, slot). Reproducible without pulling
    // in `rand`, and avoids the corner-most voxels (which the shader
    // clamps and which therefore tell us the least).
    let mut probe_positions: Vec<Vec3> = Vec::new();
    let mut expected_voxels: Vec<(usize, u32)> = Vec::new();
    for &cell_idx in &allocated_cells {
        let cell_min = cell_min_world(cell_idx, bounds);
        let mut state: u32 = cell_idx.wrapping_mul(0x9E3779B1).wrapping_add(1);
        for _ in 0..50 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let voxel_linear = state % SUBGRID_VOXELS;
            let vz = voxel_linear / (SUBGRID_DIM * SUBGRID_DIM);
            let vy = (voxel_linear / SUBGRID_DIM) % SUBGRID_DIM;
            let vx = voxel_linear % SUBGRID_DIM;
            let voxel_offset =
                Vec3::new(vx as f32, vy as f32, vz as f32) / (SUBGRID_DIM as f32);
            probe_positions.push(cell_min + voxel_offset * cell_size);
            expected_voxels.push((cell_idx as usize, voxel_linear));
        }
    }

    let run = run_lookup_probes(
        &device, &queue, &sampler, bounds, 1024, &probe_positions,
    );
    let eps = 1e-5_f32;
    for (i, &(cell_idx, voxel_linear)) in expected_voxels.iter().enumerate() {
        let subgrid_idx = run.root_indices[cell_idx];
        assert!(subgrid_idx < 1024, "cell {cell_idx} should be allocated");
        let pool_idx = (subgrid_idx as usize) * (SUBGRID_VOXELS as usize)
            + voxel_linear as usize;
        let expected = run.subgrid_pool[pool_idx];
        let actual = run.results[i];
        assert!(
            (actual - expected).abs() < eps,
            "probe {i} cell {cell_idx} voxel {voxel_linear}: GPU lookup {actual} vs pool {expected}",
        );
    }
}

#[test]
fn lookup_trilinear_midpoint_matches_corner_average() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_trilinear_midpoint_matches_corner_average: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let bounds = test_bounds();
    // Bootstrap run: discover allocated cells.
    let bootstrap = run_lookup_probes(
        &device, &queue, &sampler, bounds, 1024, &[Vec3::splat(0.0)],
    );
    let cell_idx = (0..ROOT_CELLS)
        .find(|&idx| bootstrap.root_indices[idx as usize] < 1024)
        .expect("expected at least one allocated cell");

    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let voxel_size = cell_size / (SUBGRID_DIM as f32);
    let cell_min = cell_min_world(cell_idx, bounds);

    // Sample at the midpoint between voxel(0,0,0) and voxel(1,0,0) of
    // this cell — `f.x = 0.5`, `f.y = f.z = 0`. Trilinear collapses
    // to `0.5 * (s_v000 + s_v100)`.
    let v000_world = cell_min;
    let midpoint = cell_min + Vec3::new(0.5, 0.0, 0.0) * voxel_size;
    let v100_world = cell_min + Vec3::new(1.0, 0.0, 0.0) * voxel_size;

    let run = run_lookup_probes(
        &device, &queue, &sampler, bounds, 1024,
        &[v000_world, v100_world, midpoint],
    );
    let s000 = run.results[0];
    let s100 = run.results[1];
    let mid = run.results[2];
    let expected = 0.5 * (s000 + s100);
    let eps = 1e-5_f32;
    assert!(
        (mid - expected).abs() < eps,
        "trilinear midpoint {mid} vs corner average {expected}",
    );
}

#[test]
fn lookup_in_empty_cell_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_in_empty_cell_returns_far_sentinel: no GPU");
        return;
    };
    // Sphere parked at the low corner — radius small enough that the
    // far corner of the chunk has no surface anywhere near it. Cell
    // (15, 15, 15) at world centre `Vec3::splat(60)` is therefore
    // empty, and lookup at `Vec3::splat(56)` (centre of that cell) is
    // expected to return `2 * cell_size = 8.0`.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(8.0), 4.0);
    let bounds = test_bounds();
    let probe = Vec3::splat(56.0);
    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 256, &[probe]);

    // Sanity: the cell containing `probe` is actually empty.
    let cell = ((probe - bounds.min)
        / ((bounds.max - bounds.min) / ROOT_DIM as f32))
        .floor();
    let cell_idx = (cell.x as u32)
        + (cell.y as u32) * ROOT_DIM
        + (cell.z as u32) * ROOT_DIM * ROOT_DIM;
    assert_eq!(
        run.root_indices[cell_idx as usize], 0xFFFFFFFFu32,
        "test scene mis-tuned: probe cell should be empty",
    );

    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected);
}

#[test]
fn lookup_out_of_bounds_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_out_of_bounds_returns_far_sentinel: no GPU");
        return;
    };
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let bounds = test_bounds();
    let below = bounds.min - Vec3::splat(1.0);
    let above = bounds.max + Vec3::splat(1.0);
    let run = run_lookup_probes(
        &device, &queue, &sampler, bounds, 256, &[below, above],
    );
    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected, "below bounds_min must return far");
    assert_eq!(run.results[1], expected, "above bounds_max must return far");
}

#[test]
fn lookup_in_alloc_failed_cell_returns_far_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping lookup_in_alloc_failed_cell_returns_far_sentinel: no GPU");
        return;
    };
    // Pool capacity 4 with the 64³ sphere/16 scene → ~780 cells
    // marked, only 4 succeed. Find one that landed on
    // ALLOC_FAILED_SENTINEL and probe its centre.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let bounds = test_bounds();
    let bootstrap = run_lookup_probes(
        &device, &queue, &sampler, bounds, 4, &[Vec3::splat(0.0)],
    );
    let failed_cell = (0..ROOT_CELLS)
        .find(|&idx| bootstrap.root_indices[idx as usize] == ALLOC_FAILED_SENTINEL)
        .expect("expected at least one ALLOC_FAILED cell with capacity 4");
    let cell_size = (bounds.max - bounds.min) / ROOT_DIM as f32;
    let cz = failed_cell / (ROOT_DIM * ROOT_DIM);
    let cy = (failed_cell / ROOT_DIM) % ROOT_DIM;
    let cx = failed_cell % ROOT_DIM;
    let probe = bounds.min
        + Vec3::new(cx as f32, cy as f32, cz as f32) * cell_size
        + cell_size * 0.5;

    let run = run_lookup_probes(&device, &queue, &sampler, bounds, 4, &[probe]);
    let expected = cell_size.x.max(cell_size.y).max(cell_size.z) * 2.0;
    assert_eq!(run.results[0], expected);
    // Discard `grid` ownership warning — read indirectly via the run.
    let _ = run.grid;
}
