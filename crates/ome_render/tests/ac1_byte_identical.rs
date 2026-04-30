//! AC1 — byte-identical determinism over the OmeAccel single-chunk path.
//!
//! Two consecutive runs of `eval_scene_bvh` over the **same** scene +
//! the **same** sample points must produce bit-identical f32 output.
//! Proves the per-role accumulator visit order is a function of the
//! BLAS topology only (never of runtime ray geometry), so PR-2's
//! migration to the pool-driven shader doesn't introduce a new source
//! of non-determinism vs the legacy global-BVH path.
//!
//! Skipped when no Vulkan adapter is available — same policy as the
//! sibling `pool_eval_smoke.rs` smoke test.

use bytemuck::{Pod, Zeroable};
use ome_bvh::{
    AccelCaps, ChunkInsert, IS_RAYMARCH, LeafAabb, OmeAccel, ROLE_RAYMARCH_ADD,
};
use ome_render::raymarch::POOL_EVAL_SHADER_SOURCE;
use wgpu::util::DeviceExt;

const TYPE_SPHERE: u32 = 0;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug)]
struct SmokePrimitive {
    position: [f32; 3],
    type_tag: u32,
    rotation: [f32; 4],
    scale: [f32; 3],
    smoothness: f32,
    params: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug)]
struct SamplePoint {
    p: [f32; 4],
}

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("raymarch_ac1_byte_identical"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

/// Linear-congruential generator — same shape the legacy gpu_tests
/// used so reproductions match across the migration. Returns
/// `f32 ∈ [0, 1)`.
fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

fn random_sphere_scene(n: u32, seed: u32) -> (Vec<SmokePrimitive>, Vec<LeafAabb>) {
    let mut state = seed;
    let mut prims = Vec::with_capacity(n as usize);
    let mut leaves = Vec::with_capacity(n as usize);
    for i in 0..n {
        let pos = [
            (lcg(&mut state) - 0.5) * 100.0,
            (lcg(&mut state) - 0.5) * 100.0,
            (lcg(&mut state) - 0.5) * 100.0,
        ];
        let radius = 0.5 + lcg(&mut state) * 0.5;
        let prim = SmokePrimitive {
            position: pos,
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [radius, 0.0, 0.0, 0.0],
        };
        leaves.push(LeafAabb {
            aabb_min: [pos[0] - radius, pos[1] - radius, pos[2] - radius],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [pos[0] + radius, pos[1] + radius, pos[2] + radius],
            entity_id: i,
        });
        prims.push(prim);
    }
    (prims, leaves)
}

/// Sample-point grid in `[-50, 50]^3` — wider than the scene's
/// `[-50, 50]` placement envelope so the run hits both inside-AABB
/// and outside-AABB code paths.
fn sample_points_grid(n: u32) -> Vec<SamplePoint> {
    let mut state = 0xdeadbeef_u32;
    (0..n)
        .map(|_| {
            let x = (lcg(&mut state) - 0.5) * 100.0;
            let y = (lcg(&mut state) - 0.5) * 100.0;
            let z = (lcg(&mut state) - 0.5) * 100.0;
            SamplePoint { p: [x, y, z, 0.0] }
        })
        .collect()
}

struct CullSetup {
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

fn build_cull_setup(device: &wgpu::Device) -> CullSetup {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ac1_byte_identical_module"),
        source: wgpu::ShaderSource::Wgsl(POOL_EVAL_SHADER_SOURCE.into()),
    });
    let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ac1_bgl_io"),
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
        label: Some("ac1_bgl_pool"),
        entries: &pool_entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ac1_pipeline_layout"),
        bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ac1_pipeline"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("cs_eval_smoke"),
        compilation_options: Default::default(),
        cache: None,
    });
    CullSetup { bgl0, bgl1, pipeline }
}

/// Dispatch `cs_eval_smoke` over `samples` against the pool resident
/// in `accel`, read back `f32` distances. Encapsulates the per-run
/// boilerplate so the test can re-run easily.
fn run_eval_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    setup: &CullSetup,
    accel: &OmeAccel,
    samples: &[SamplePoint],
) -> Vec<f32> {
    let n = samples.len() as u32;
    let sample_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ac1_sample_points"),
        contents: bytemuck::cast_slice(samples),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let dist_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ac1_sample_distances"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ac1_bg_io"),
        layout: &setup.bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: sample_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: dist_buf.as_entire_binding() },
        ],
    });
    let bg1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ac1_bg_pool"),
        layout: &setup.bgl1,
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
        label: Some("ac1_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ac1_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&setup.pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &bg1, &[]);
        pass.dispatch_workgroups((n + 63) / 64, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ac1_staging"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc2 = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ac1_readback_encoder"),
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

/// Two consecutive eval passes over the same scene + same samples
/// must produce bit-identical output. AC1 of issue #360 — exercises
/// the determinism of the BLAS topology + the per-role accumulator
/// visit order through the OmeAccel single-chunk path.
fn run_byte_identical_at(n_primitives: u32, seed: u32, n_samples: u32) {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("ac1_byte_identical: no adapter — skipping");
        return;
    };
    let (prims, leaves) = random_sphere_scene(n_primitives, seed);
    let primitives_bytes = bytemuck::cast_slice::<_, u8>(&prims).to_vec();

    let mut accel = OmeAccel::new(
        &device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();
    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 0,
                leaf_aabbs: &leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu(&queue, 0.0, 0.0);

    let setup = build_cull_setup(&device);
    let samples = sample_points_grid(n_samples);

    let run_a = run_eval_pass(&device, &queue, &setup, &accel, &samples);
    let run_b = run_eval_pass(&device, &queue, &setup, &accel, &samples);

    assert_eq!(run_a.len(), run_b.len());
    for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
        // Bit-exact equality: `to_bits` makes the determinism intent
        // explicit against future readers (NaN payloads + signed-zero
        // are bit-distinguishable but `==` would conflate them).
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "AC1 sample[{i}] diverged across runs at N={n_primitives}: {a} vs {b}",
        );
    }
}

#[test]
fn ac1_byte_identical_n_8() {
    run_byte_identical_at(8, 0xc0ffee01, 512);
}

#[test]
fn ac1_byte_identical_n_1024() {
    // BVH at this size has multiple internal levels — catches
    // non-determinism in stack push ordering or accumulator order
    // that smaller cases miss.
    run_byte_identical_at(1024, 0xfeedface, 2048);
}
