//! Compute-kernel smoke test for the pool-driven raymarch shader
//! introduced in PR-1 of #360. Drives `cs_eval_smoke` over an
//! `OmeAccel` populated with a single chunk of three SDF spheres,
//! reads back the GPU distances, and compares them to a CPU mirror
//! that walks the same primitives in the same per-role fold.
//!
//! Skipped when no adapter is available — same policy as the existing
//! GPU test harness in `crates/ome_render/src/raymarch/bvh/gpu_tests/
//! harness.rs`. The PR-1 smoke proxy for AC1 byte-identical: the
//! raymarcher runtime is not yet wired to `OmeAccel` (PR-2), so the
//! shader is exercised in isolation through its compute entry point.

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use ome_bvh::{
    AccelCaps, ChunkInsert, IS_RAYMARCH, LeafAabb, OmeAccel, ROLE_RAYMARCH_ADD,
};
use ome_render::raymarch::POOL_EVAL_SHADER_SOURCE;
use wgpu::util::DeviceExt;

const TYPE_SPHERE: u32 = 0;

/// Mirror of the WGSL `SdfPrimitive` — `smoothness` lives in the
/// legacy `_pad0` slot so the byte layout matches.
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

/// 16-byte stride for the WGSL `array<vec4<f32>>` `sample_points`.
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
        label: Some("raymarch_pool_eval_smoke"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

fn quat_rotate_inv(q: [f32; 4], v: Vec3) -> Vec3 {
    // Matches `transform_point(p, position, rotation)` in
    // `sdf_primitives.wgsl`: rotates by the conjugate of `rotation`.
    let inv = Quat::from_xyzw(-q[0], -q[1], -q[2], q[3]);
    inv.mul_vec3(v)
}

fn sdf_sphere_cpu(p: Vec3, radius: f32) -> f32 {
    p.length() - radius
}

fn smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h - k * h * (1.0 - h)
}

fn smooth_intersection(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 - d1) / k).clamp(0.0, 1.0);
    d2 * (1.0 - h) + d1 * h + k * h * (1.0 - h)
}

fn smooth_subtraction(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (0.5 - 0.5 * (d2 + d1) / k).clamp(0.0, 1.0);
    d1 * (1.0 - h) + (-d2) * h + k * h * (1.0 - h)
}

fn eval_primitive_at_cpu(p: Vec3, prim: &SmokePrimitive) -> f32 {
    let scale = Vec3::from_array(prim.scale).max(Vec3::splat(1e-5));
    let local = quat_rotate_inv(prim.rotation, p - Vec3::from_array(prim.position)) / scale;
    let s_min = scale.min_element();
    sdf_sphere_cpu(local, prim.params[0]) * s_min
}

// Mirror the WGSL identities. See `raymarch_pool_eval.wgsl` for why
// `±1e6` instead of `±1e10` — keeps the identity collapse precise on
// every Vulkan backend, including the radv `mix(a, b, t) = a + (b-a)*t`
// implementation that loses precision at extreme magnitudes.
const ACC_UNION_IDENTITY: f32 = 1.0e6;
const ACC_INTERSECT_IDENTITY: f32 = -1.0e6;

/// CPU mirror of `eval_scene_bvh` for one chunk. Walks `prims` in
/// input order with the same `IS_RAYMARCH` + AABB-contains gates the
/// GPU runs at the BLAS-leaf step.
fn eval_scene_cpu(
    p: Vec3,
    prims: &[SmokePrimitive],
    leaves: &[LeafAabb],
    k_int_global: f32,
    k_sub_global: f32,
) -> f32 {
    let mut acc_add = ACC_UNION_IDENTITY;
    let mut acc_int = ACC_INTERSECT_IDENTITY;
    let mut acc_sub = ACC_UNION_IDENTITY;
    for (prim, leaf) in prims.iter().zip(leaves.iter()) {
        if (leaf.flags & IS_RAYMARCH) == 0 {
            continue;
        }
        let lo = Vec3::from_array(leaf.aabb_min);
        let hi = Vec3::from_array(leaf.aabb_max);
        let inside = (p.x >= lo.x)
            && (p.y >= lo.y)
            && (p.z >= lo.z)
            && (p.x <= hi.x)
            && (p.y <= hi.y)
            && (p.z <= hi.z);
        if !inside {
            continue;
        }
        let d = eval_primitive_at_cpu(p, prim);
        let k = prim.smoothness.max(1e-5);
        match leaf.flags & 0x3 {
            0 => acc_add = smooth_union(acc_add, d, k),
            1 => acc_int = smooth_intersection(acc_int, d, k),
            2 => acc_sub = smooth_union(acc_sub, d, k),
            _ => acc_add = smooth_union(acc_add, d, k),
        }
    }
    let k_int = k_int_global.max(1e-5);
    let k_sub = k_sub_global.max(1e-5);
    let r = smooth_intersection(acc_add, acc_int, k_int);
    smooth_subtraction(r, acc_sub, k_sub)
}

#[test]
fn cs_eval_smoke_matches_cpu_mirror_for_single_chunk() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("skipping cs_eval_smoke — no adapter");
        return;
    };

    // Three add-role spheres along x. AABBs inflated by `K_LEAF` so
    // the BLAS / TLAS culls stay conservative under the smooth blend.
    const RADIUS: f32 = 1.0;
    const K_LEAF: f32 = 0.5;
    let mut prims = Vec::new();
    let mut leaves = Vec::new();
    for (i, x) in [-2.0_f32, 0.0, 2.0].into_iter().enumerate() {
        prims.push(SmokePrimitive {
            position: [x, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: K_LEAF,
            params: [RADIUS, 0.0, 0.0, 0.0],
        });
        leaves.push(LeafAabb {
            aabb_min: [x - RADIUS - K_LEAF, -RADIUS - K_LEAF, -RADIUS - K_LEAF],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [x + RADIUS + K_LEAF, RADIUS + K_LEAF, RADIUS + K_LEAF],
            entity_id: i as u32,
        });
    }
    let primitives_bytes = bytemuck::cast_slice::<_, u8>(&prims).to_vec();

    let mut accel = OmeAccel::new(
        &device,
        AccelCaps::TEST,
        std::mem::size_of::<SmokePrimitive>() as u32,
    )
    .unwrap();

    accel
        .insert_chunk(
            &queue,
            ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: K_LEAF,
            },
        )
        .unwrap();

    let k_int_global: f32 = 0.5;
    let k_sub_global: f32 = 0.5;
    accel.update_gpu(&queue, k_int_global, k_sub_global);

    // Sample points at varied distances — inside one sphere, in the
    // smooth-blend zone between two spheres, and well outside.
    let sample_xs: [f32; 8] = [-3.5, -2.0, -1.0, 0.0, 0.5, 1.5, 2.0, 4.0];
    let samples: Vec<SamplePoint> = sample_xs
        .iter()
        .map(|x| SamplePoint { p: [*x, 0.0, 0.0, 0.0] })
        .collect();
    let n = samples.len() as u32;

    let sample_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("smoke_sample_points"),
        contents: bytemuck::cast_slice(&samples),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let dist_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke_sample_distances"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("raymarch_pool_eval_smoke_module"),
        source: wgpu::ShaderSource::Wgsl(POOL_EVAL_SHADER_SOURCE.into()),
    });

    let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("smoke_bgl_io"),
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

    let pool_layout_entries: Vec<wgpu::BindGroupLayoutEntry> = (5..=9u32)
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
        label: Some("smoke_bgl_pool"),
        entries: &pool_layout_entries,
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("smoke_pipeline_layout"),
        bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
        immediate_size: 0,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("smoke_pipeline"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("cs_eval_smoke"),
        compilation_options: Default::default(),
        cache: None,
    });

    let bg0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("smoke_bg_io"),
        layout: &bgl0,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: sample_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: dist_buf.as_entire_binding(),
            },
        ],
    });

    let bg1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("smoke_bg_pool"),
        layout: &bgl1,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: accel.buffers.tlas_nodes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: accel.buffers.chunk_descriptors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: accel.buffers.bvh_nodes_pool.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: accel.buffers.leaf_aabbs_pool.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: accel.buffers.primitives_pool.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: accel.buffers.tlas_uniforms.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("smoke_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("smoke_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg0, &[]);
        pass.set_bind_group(1, &bg1, &[]);
        pass.dispatch_workgroups((n + 63) / 64, 1, 1);
    }
    queue.submit(std::iter::once(encoder.finish()));

    // Readback.
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke_staging"),
        size: (n as u64) * 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc2 = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("smoke_readback_encoder"),
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
    let gpu: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();

    // Compare GPU vs CPU mirror.
    for (i, x) in sample_xs.iter().enumerate() {
        let p = Vec3::new(*x, 0.0, 0.0);
        let cpu = eval_scene_cpu(p, &prims, &leaves, k_int_global, k_sub_global);
        let g = gpu[i];
        let diff = (g - cpu).abs();
        let rel = diff / cpu.abs().max(1e-3);
        // 1e-4 absolute / 1e-4 relative — generous to cover the
        // smooth_union non-associativity from a different fold order
        // between CPU mirror and morton-sorted BLAS traversal. The
        // key assertion is a tracking match, not bit-identity (PR-2
        // ships the AC1 bit-identity test against the live raymarcher).
        assert!(
            diff < 1e-4 || rel < 1e-4,
            "sample {i} at x={x}: gpu={g} cpu={cpu} diff={diff} rel={rel}",
        );
    }
}
