//! AC6 — load-order determinism over the OmeAccel single-chunk path.
//!
//! The same primitive set inserted into `OmeAccel` in two different
//! input orders must produce bit-identical sample output. The pool's
//! BVH is morton-sorted by centroid (deterministic, position-only),
//! so the topology and the per-role visit order are functions of
//! the **scene** rather than the **input ordering**. PR-3 generalises
//! the test to multi-chunk insertions; PR-2's single-chunk version
//! pins the per-chunk invariant in isolation.
//!
//! Skipped when no Vulkan adapter is available.

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
        label: Some("raymarch_ac6_load_order"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

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

fn sample_points_grid(n: u32) -> Vec<SamplePoint> {
    let mut state = 0xc0ffe_caf_u32;
    (0..n)
        .map(|_| {
            let x = (lcg(&mut state) - 0.5) * 100.0;
            let y = (lcg(&mut state) - 0.5) * 100.0;
            let z = (lcg(&mut state) - 0.5) * 100.0;
            SamplePoint { p: [x, y, z, 0.0] }
        })
        .collect()
}

/// Reproducible Fisher-Yates shuffle of `(prims, leaves)` so the two
/// arrays stay aligned 1:1 after the permutation.
fn shuffle_aligned(
    prims: &mut [SmokePrimitive],
    leaves: &mut [LeafAabb],
    seed: u32,
) {
    debug_assert_eq!(prims.len(), leaves.len());
    let mut state = seed;
    for i in (1..prims.len()).rev() {
        let j = (lcg(&mut state) * (i as f32 + 1.0)) as usize;
        let j = j.min(i);
        prims.swap(i, j);
        leaves.swap(i, j);
    }
}

fn dispatch_eval_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    accel: &OmeAccel,
    samples: &[SamplePoint],
) -> Vec<f32> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ac6_module"),
        source: wgpu::ShaderSource::Wgsl(POOL_EVAL_SHADER_SOURCE.into()),
    });
    let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ac6_bgl_io"),
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
        label: Some("ac6_bgl_pool"),
        entries: &pool_entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ac6_pipeline_layout"),
        bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ac6_pipeline"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("cs_eval_smoke"),
        compilation_options: Default::default(),
        cache: None,
    });

    let n = samples.len() as u32;
    let sample_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ac6_sample_points"),
        contents: bytemuck::cast_slice(samples),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let dist_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ac6_sample_distances"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ac6_bg_io"),
        layout: &bgl0,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: sample_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: dist_buf.as_entire_binding() },
        ],
    });
    let bg1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ac6_bg_pool"),
        layout: &bgl1,
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
        label: Some("ac6_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ac6_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &bg1, &[]);
        pass.dispatch_workgroups((n + 63) / 64, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ac6_staging"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc2 = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ac6_readback_encoder"),
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

fn populate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    prims: &[SmokePrimitive],
    leaves: &[LeafAabb],
) -> OmeAccel {
    let mut accel = OmeAccel::new(
        device,
        AccelCaps::default(),
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();
    let primitives_bytes = bytemuck::cast_slice::<_, u8>(prims).to_vec();
    accel
        .insert_chunk(
            queue,
            ChunkInsert {
                key: 0,
                leaf_aabbs: leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    accel.update_gpu_standalone(device, queue, 0.0, 0.0);
    accel
}

#[test]
fn ac6_load_order_determinism_n_64() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("ac6_load_order: no adapter — skipping");
        return;
    };
    let (mut prims_a, mut leaves_a) = random_sphere_scene(64, 0xa6_aabb_01);
    let mut prims_b = prims_a.clone();
    let mut leaves_b = leaves_a.clone();
    // Shuffle (prims_b, leaves_b) — same set of primitives, different
    // input order. The resulting BVH topology is morton-sorted by
    // centroid so it must be identical to the un-shuffled case.
    shuffle_aligned(&mut prims_b, &mut leaves_b, 0xfeed_0042);

    // Sanity: shuffle actually changed the order.
    let _ = (&mut prims_a, &mut leaves_a);
    assert!(
        prims_b
            .iter()
            .zip(prims_a.iter())
            .any(|(a, b)| a.position != b.position),
        "shuffle did not change input order — test would not exercise AC6",
    );

    let accel_a = populate(&device, &queue, &prims_a, &leaves_a);
    let accel_b = populate(&device, &queue, &prims_b, &leaves_b);

    let samples = sample_points_grid(1024);
    let run_a = dispatch_eval_into(&device, &queue, &accel_a, &samples);
    let run_b = dispatch_eval_into(&device, &queue, &accel_b, &samples);

    assert_eq!(run_a.len(), run_b.len());
    for (i, (a, b)) in run_a.iter().zip(run_b.iter()).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "AC6 sample[{i}] diverged across input orderings: {a} vs {b}",
        );
    }
}
