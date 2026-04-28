//! Tests for [`crate::sparse::free_list`] — split out so the impl
//! file stays under the no-monolithic threshold. The two GPU
//! dispatch tests share helpers ([`build_kernel`], [`storage_entry`])
//! to keep boilerplate from compounding.

use super::CountersInit;
use crate::sparse::{
    ALLOC_FAILED_SENTINEL, FREELIST_COUNTERS_SIZE, SPARSE_FREELIST_WGSL, SparseGrid, test_device,
    test_device::readback,
};
use glam::Vec3;
use ome_bvh::Aabb;
use std::collections::HashSet;

fn unit_bounds() -> Aabb {
    Aabb::new(Vec3::ZERO, Vec3::splat(64.0))
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn build_kernel(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    entry_point: &str,
    bgl: &wgpu::BindGroupLayout,
) -> wgpu::ComputePipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        module: shader,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn dispatch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    pipeline: &wgpu::ComputePipeline,
    bg: &wgpu::BindGroup,
    workgroups: u32,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some(label),
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        cpass.set_pipeline(pipeline);
        cpass.set_bind_group(0, bg, &[]);
        cpass.dispatch_workgroups(workgroups, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));
}

fn read_u32s(device: &wgpu::Device, queue: &wgpu::Queue, src: &wgpu::Buffer) -> Vec<u32> {
    readback(device, queue, src)
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

const POP_KERNEL_TAIL: &str = "
@group(0) @binding(2) var<storage, read_write> popped: array<u32>;

@compute @workgroup_size(64)
fn pop_n(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&popped)) { return; }
    popped[i] = sparse_pop_subgrid_index();
}

@compute @workgroup_size(64)
fn push_back(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&popped)) { return; }
    sparse_push_subgrid_index(popped[i]);
}
";

#[test]
fn helpers_wgsl_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(SPARSE_FREELIST_WGSL)
        .expect("sparse_freelist.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("sparse_freelist.wgsl should validate");
}

#[test]
fn helpers_wgsl_exposes_pop_and_push() {
    for name in ["sparse_pop_subgrid_index", "sparse_push_subgrid_index"] {
        assert!(
            SPARSE_FREELIST_WGSL.contains(&format!("fn {name}(")),
            "missing function `{name}` in sparse_freelist.wgsl",
        );
    }
    assert!(SPARSE_FREELIST_WGSL.contains("const SPARSE_ALLOC_FAILED"));
    assert!(SPARSE_FREELIST_WGSL.contains("struct SparseCounters"));
}

#[test]
fn counters_size_matches_wgsl_layout() {
    assert_eq!(
        FREELIST_COUNTERS_SIZE,
        std::mem::size_of::<CountersInit>() as u64,
    );
    assert_eq!(FREELIST_COUNTERS_SIZE, 16);
}

#[test]
fn init_writes_identity_permutation_and_counters() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping init_writes_identity_permutation_and_counters: no GPU");
        return;
    };
    let max_subgrids = 32;
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

    let free = read_u32s(&device, &queue, grid.free_list_buffer());
    assert_eq!(free.len(), max_subgrids as usize);
    for (i, val) in free.iter().enumerate() {
        assert_eq!(*val, i as u32, "free_list[{i}] must be {i}, got {val}");
    }

    let counters = read_u32s(&device, &queue, grid.counters_buffer());
    assert_eq!(
        counters,
        vec![max_subgrids, 0, 0, 0],
        "counters: free_top + alloc_failed + 2 pad",
    );
}

/// End-to-end: pop every available index in parallel, verify the
/// popped set is exactly `0..max_subgrids` (no duplicates, no
/// misses), then push every index back and verify the counters
/// return to baseline.
#[test]
fn pop_then_push_round_trip_exhausts_and_restores_pool() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping pop_then_push_round_trip_exhausts_and_restores_pool: no GPU");
        return;
    };
    let max_subgrids: u32 = 64;
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

    let popped_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::popped"),
        size: (max_subgrids as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let shader_src = format!("{SPARSE_FREELIST_WGSL}{POP_KERNEL_TAIL}");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test::sparse_freelist_round_trip"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test::bgl"),
        entries: &[storage_entry(0), storage_entry(1), storage_entry(2)],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: grid.free_list_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.counters_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: popped_buffer.as_entire_binding(),
            },
        ],
    });
    let pop_pipeline = build_kernel(&device, "test::pop", &shader, "pop_n", &bgl);
    let push_pipeline = build_kernel(&device, "test::push", &shader, "push_back", &bgl);

    dispatch(
        &device,
        &queue,
        "test::pop_dispatch",
        &pop_pipeline,
        &bg,
        max_subgrids.div_ceil(64),
    );

    let popped = read_u32s(&device, &queue, &popped_buffer);
    let counters = read_u32s(&device, &queue, grid.counters_buffer());
    assert_eq!(counters[0], 0, "free_top must be 0 after pop-all");
    assert_eq!(counters[1], 0, "no allocation should have failed");
    let popped_set: HashSet<u32> = popped.iter().copied().collect();
    assert_eq!(popped_set.len(), max_subgrids as usize);
    for i in 0..max_subgrids {
        assert!(popped_set.contains(&i), "missing index {i} in popped set");
    }

    dispatch(
        &device,
        &queue,
        "test::push_dispatch",
        &push_pipeline,
        &bg,
        max_subgrids.div_ceil(64),
    );

    let counters = read_u32s(&device, &queue, grid.counters_buffer());
    assert_eq!(counters[0], max_subgrids, "free_top must restore to max");
    assert_eq!(counters[1], 0, "alloc_failed_count must remain 0");

    let free_after: HashSet<u32> = read_u32s(&device, &queue, grid.free_list_buffer())
        .into_iter()
        .collect();
    assert_eq!(
        free_after.len(),
        max_subgrids as usize,
        "free_list contents must remain a permutation of 0..max_subgrids",
    );
}

/// Pop more times than the pool has slots: extra pops must report
/// `SPARSE_ALLOC_FAILED` and increment `alloc_failed_count` by
/// exactly that count.
#[test]
fn pop_underflow_reports_alloc_failed_sentinel() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("skipping pop_underflow_reports_alloc_failed_sentinel: no GPU");
        return;
    };
    let max_subgrids: u32 = 16;
    let attempts: u32 = 32; // 16 successful + 16 failed
    let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

    let popped_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test::popped_underflow"),
        size: (attempts as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let shader_src = format!("{SPARSE_FREELIST_WGSL}{POP_KERNEL_TAIL}");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test::sparse_freelist_underflow"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test::bgl_underflow"),
        entries: &[storage_entry(0), storage_entry(1), storage_entry(2)],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test::bg_underflow"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: grid.free_list_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.counters_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: popped_buffer.as_entire_binding(),
            },
        ],
    });
    let pipeline = build_kernel(&device, "test::pop_underflow", &shader, "pop_n", &bgl);

    dispatch(
        &device,
        &queue,
        "test::underflow_dispatch",
        &pipeline,
        &bg,
        attempts.div_ceil(64),
    );

    let popped = read_u32s(&device, &queue, &popped_buffer);
    let counters = read_u32s(&device, &queue, grid.counters_buffer());

    let succeeded: HashSet<u32> = popped
        .iter()
        .copied()
        .filter(|v| *v != ALLOC_FAILED_SENTINEL)
        .collect();
    let failed_count = popped
        .iter()
        .filter(|v| **v == ALLOC_FAILED_SENTINEL)
        .count() as u32;

    assert_eq!(
        succeeded.len(),
        max_subgrids as usize,
        "exactly max_subgrids pops must succeed with unique indices",
    );
    for i in 0..max_subgrids {
        assert!(succeeded.contains(&i));
    }
    assert_eq!(
        failed_count,
        attempts - max_subgrids,
        "remaining pops must return ALLOC_FAILED_SENTINEL",
    );
    assert_eq!(counters[0], 0, "free_top must remain at 0");
    assert_eq!(
        counters[1],
        attempts - max_subgrids,
        "alloc_failed_count must equal the number of failed pops",
    );
}
