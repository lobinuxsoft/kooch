//! Tests for [`crate::sparse::classify`] — kept in their own file so
//! the impl module stays under the no-monolithic threshold. The GPU
//! tests share helpers ([`run_classify`], [`cpu_classify`]) so each
//! test stays focused on the assertion it makes.
//!
//! Every GPU test gates on [`test_device::try_acquire`] and skips
//! cleanly when no adapter is available — CI without a display, or a
//! sandbox without GPU passthrough, is expected.

use super::{
    CLASSIFY_FINALIZE_WGSL, CLASSIFY_WGSL, CLASSIFY_WORKGROUP_SIZE, ClassifyPass, DEFAULT_MARGIN,
};
use crate::sparse::{
    ANALYTIC_SPHERE_WGSL, AnalyticSphereSampler, ROOT_CELLS, ROOT_DIM, SdfSampler, SparseGrid,
    test_device,
};
use glam::Vec3;
use ome_bvh::Aabb;
use std::collections::HashSet;

const TEST_BOUNDS_MIN: Vec3 = Vec3::ZERO;
const TEST_BOUNDS_MAX: Vec3 = Vec3::splat(64.0);

fn test_bounds() -> Aabb {
    Aabb::new(TEST_BOUNDS_MIN, TEST_BOUNDS_MAX)
}

/// CPU mirror of the WGSL Lipschitz cone test in `sparse_classify.wgsl`.
/// Returns the linear root-cell indices the GPU pass is expected to
/// mark when given `sampler` over `bounds` at `margin`.
fn cpu_classify(sampler: &AnalyticSphereSampler, bounds: Aabb, margin: f32) -> Vec<u32> {
    let extent = bounds.max - bounds.min;
    let cell_size = extent / (ROOT_DIM as f32);
    let cell_diag = cell_size.length();
    let mut marks = Vec::new();
    for cell_idx in 0..ROOT_CELLS {
        let cz = cell_idx / (ROOT_DIM * ROOT_DIM);
        let cy = (cell_idx / ROOT_DIM) % ROOT_DIM;
        let cx = cell_idx % ROOT_DIM;
        let cell_3d = Vec3::new(cx as f32, cy as f32, cz as f32);
        let center = bounds.min + (cell_3d + Vec3::splat(0.5)) * cell_size;
        let sdf = sampler.sample_cpu(center);
        if sdf.abs() < cell_diag * margin {
            marks.push(cell_idx);
        }
    }
    marks
}

/// Run one `ClassifyPass::record` against `sampler` and read back
/// `(needs_count, needs_indices[0..needs_count], indirect_args)`.
fn run_classify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sampler: &AnalyticSphereSampler,
    margin: f32,
) -> (u32, Vec<u32>, [u32; 3]) {
    let grid = SparseGrid::new(device, queue, test_bounds(), 256);
    let pass = ClassifyPass::new(
        device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::sampler_bg"),
        layout: pass.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::classify_encoder"),
    });
    pass.record(device, queue, &mut encoder, &grid, &sampler_bg, margin);
    queue.submit(std::iter::once(encoder.finish()));

    let count_bytes = test_device::readback(device, queue, grid.needs_count_buffer());
    let count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);

    let indices_bytes = test_device::readback(device, queue, grid.needs_indices_buffer());
    let mut indices: Vec<u32> = indices_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    indices.truncate(count as usize);

    let args_bytes = test_device::readback(device, queue, grid.needs_indirect_args_buffer());
    let args = [
        u32::from_le_bytes([args_bytes[0], args_bytes[1], args_bytes[2], args_bytes[3]]),
        u32::from_le_bytes([args_bytes[4], args_bytes[5], args_bytes[6], args_bytes[7]]),
        u32::from_le_bytes([args_bytes[8], args_bytes[9], args_bytes[10], args_bytes[11]]),
    ];

    (count, indices, args)
}

#[test]
fn classify_concat_parses_and_validates() {
    // Build the exact WGSL the pipeline would compile — sampler
    // fragment + classify body — and run naga's parse + validate so a
    // copy-paste regression in either file fails fast.
    let combined = format!("{ANALYTIC_SPHERE_WGSL}{CLASSIFY_WGSL}");
    let module = naga::front::wgsl::parse_str(&combined)
        .expect("classify concat should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("classify concat should validate");
}

#[test]
fn classify_finalize_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(CLASSIFY_FINALIZE_WGSL)
        .expect("classify finalize should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("classify finalize should validate");
}

#[test]
fn classify_wgsl_constants_match_host() {
    // The shader carries its own copies of `ROOT_DIM` / `ROOT_CELLS`
    // for inlining; this guard fails fast if either drifts from the
    // host-side constants in `super::sparse`.
    assert!(
        CLASSIFY_WGSL.contains(&format!("CLASSIFY_ROOT_DIM: u32 = {ROOT_DIM}u")),
        "CLASSIFY_ROOT_DIM in shader must mirror sparse::ROOT_DIM",
    );
    assert!(
        CLASSIFY_WGSL.contains(&format!("CLASSIFY_ROOT_CELLS: u32 = {ROOT_CELLS}u")),
        "CLASSIFY_ROOT_CELLS in shader must mirror sparse::ROOT_CELLS",
    );
}

#[test]
fn empty_grid_returns_zero_count() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping empty_grid_returns_zero_count: no GPU");
        return;
    };
    // Sphere parked far outside the chunk → |sdf| ≫ cell_diagonal for
    // every cell centre, so nothing should be marked. Cheaper than
    // adding a second sampler implementation just for this test.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(10_000.0), 1.0);
    let (count, indices, args) = run_classify(&device, &queue, &sampler, DEFAULT_MARGIN);

    assert_eq!(count, 0, "no cell should be flagged when surface is absent");
    assert!(indices.is_empty());
    assert_eq!(
        args,
        [0u32, 1u32, 1u32],
        "indirect args x must be ceil_div(0, 64) = 0",
    );
}

#[test]
fn sphere_marks_surface_cells() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping sphere_marks_surface_cells: no GPU");
        return;
    };
    // Sphere centred in the chunk with radius 16 — its surface threads
    // through the middle of the 64³ root grid (cell_size = 4.0, so the
    // shell crosses several rings of cells along each axis).
    let center = Vec3::splat(32.0);
    let radius = 16.0;
    let sampler = AnalyticSphereSampler::new(&device, center, radius);

    let (count, gpu_indices, args) = run_classify(&device, &queue, &sampler, DEFAULT_MARGIN);
    let cpu_marks = cpu_classify(&sampler, test_bounds(), DEFAULT_MARGIN);

    assert!(
        !cpu_marks.is_empty(),
        "test scene mis-tuned: CPU expected at least one marked cell",
    );
    assert_eq!(
        count as usize,
        cpu_marks.len(),
        "GPU needs_count must match CPU brute-force mark count",
    );

    let cpu_set: HashSet<u32> = cpu_marks.iter().copied().collect();
    let gpu_set: HashSet<u32> = gpu_indices.iter().copied().collect();
    assert_eq!(
        gpu_set.len(),
        count as usize,
        "needs_indices entries must be unique",
    );
    assert_eq!(
        gpu_set, cpu_set,
        "GPU and CPU mark sets must match cell-for-cell",
    );

    let expected_x = count.div_ceil(CLASSIFY_WORKGROUP_SIZE);
    assert_eq!(
        args,
        [expected_x, 1, 1],
        "indirect args must encode ceil_div(count, workgroup_size)",
    );
}

const SENTINEL_SHADER: &str = r#"
struct NeedsCount {
    value: u32,
}

struct Total {
    counter: atomic<u32>,
}

@group(0) @binding(0) var<storage, read> sentinel_needs_count: NeedsCount;
@group(0) @binding(1) var<storage, read_write> sentinel_total: Total;

@compute @workgroup_size(64)
fn sentinel_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x < sentinel_needs_count.value) {
        atomicAdd(&sentinel_total.counter, 1u);
    }
}
"#;

#[test]
fn indirect_dispatch_consumes_classify_count() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping indirect_dispatch_consumes_classify_count: no GPU");
        return;
    };
    // Reuse the sphere scene — gives us a non-trivial N to exercise.
    let sampler = AnalyticSphereSampler::new(&device, Vec3::splat(32.0), 16.0);
    let grid = SparseGrid::new(&device, &queue, test_bounds(), 256);
    let pass = ClassifyPass::new(
        &device,
        sampler.wgsl_source(),
        &sampler.bind_group_layout_entries(),
    );
    let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::sampler_bg"),
        layout: pass.sampler_bind_group_layout(),
        entries: &sampler.bind_group_entries(),
    });

    let total_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::sentinel_total"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    queue.write_buffer(&total_buffer, 0, &[0u8; 4]);

    let sentinel_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test::sentinel_shader"),
        source: wgpu::ShaderSource::Wgsl(SENTINEL_SHADER.into()),
    });
    let sentinel_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test::sentinel_bgl"),
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
    let sentinel_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("test::sentinel_layout"),
        bind_group_layouts: &[Some(&sentinel_bgl)],
        immediate_size: 0,
    });
    let sentinel_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test::sentinel_pipeline"),
        layout: Some(&sentinel_layout),
        module: &sentinel_module,
        entry_point: Some("sentinel_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let sentinel_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::sentinel_bg"),
        layout: &sentinel_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: grid.needs_count_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: total_buffer.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test::indirect_encoder"),
    });
    pass.record(
        &device,
        &queue,
        &mut encoder,
        &grid,
        &sampler_bg,
        DEFAULT_MARGIN,
    );
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("test::sentinel_indirect_pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&sentinel_pipeline);
        cpass.set_bind_group(0, &sentinel_bg, &[]);
        cpass.dispatch_workgroups_indirect(grid.needs_indirect_args_buffer(), 0);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let count_bytes = test_device::readback(&device, &queue, grid.needs_count_buffer());
    let needs_count = u32::from_le_bytes([
        count_bytes[0],
        count_bytes[1],
        count_bytes[2],
        count_bytes[3],
    ]);
    assert!(
        needs_count > 0,
        "test scene mis-tuned: classify produced no marks",
    );

    let total_bytes = test_device::readback(&device, &queue, &total_buffer);
    let total = u32::from_le_bytes([
        total_bytes[0],
        total_bytes[1],
        total_bytes[2],
        total_bytes[3],
    ]);
    assert_eq!(
        total, needs_count,
        "indirect-dispatched workgroups must cover exactly needs_count threads",
    );
}
