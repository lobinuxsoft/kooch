//! Shared harness: device acquisition, deterministic scene gen,
//! sample-point grid, and the `run_eval_pass` helper that dispatches
//! the test compute kernel and reads back the per-sample output.

use bytemuck::{Pod, Zeroable};
use glam::Quat;

use super::shader::TEST_COMPUTE_WGSL;
use crate::raymarch::aabb::primitive_aabb;
use crate::raymarch::bvh::BvhState;
use crate::raymarch::instance::{RaymarchPayload, SceneMeta, SdfPrimitive, TYPE_SPHERE};
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};

/// Read N `Pod` records back from a slot-resident storage buffer.
/// Used by the regression suite (`move_propagates`, etc.) to assert
/// the slot's contents match what a kick committed; production never
/// copies these buffers. The slot factory in
/// [`super::super::slots`] sets `COPY_SRC` for exactly this path.
pub(super) fn readback_pod<T: Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    n: u32,
    label: &str,
) -> Vec<T> {
    let bytes = (n as u64) * std::mem::size_of::<T>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("raymarch_bvh::gpu_tests::readback_pod_encoder"),
    });
    encoder.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));

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
        .expect("poll");
    rx.recv().expect("map_async sender").expect("map_async result");
    let data = slice.get_mapped_range();
    let v: Vec<T> = bytemuck::cast_slice::<u8, T>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

/// Headless GPU acquisition. Returns `None` when no adapter is
/// available or the timestamp features the BvhGpuBuilder needs are
/// missing — same skip-not-fail policy as the ome_bvh GPU tests.
pub(super) fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let needs = wgpu::Features::TIMESTAMP_QUERY
            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        if !adapter.features().contains(needs) {
            return None;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("raymarch_bvh::test_device"),
                required_features: needs,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            })
            .await
            .ok()?;
        Some((device, queue))
    })
}

/// Same fixed-step LCG used by the ome_bvh tests, so reproductions
/// match.
fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    (*state >> 16) as f32 / 32768.0
}

/// Build a deterministic scene of `n` unit spheres scattered across
/// a 100³ box. Returns the per-primitive arrays the production
/// pipeline maintains in lockstep: `(primitives, leaf_aabbs,
/// raymarch_payloads)`.
pub(super) fn random_sphere_scene(
    n: u32,
    seed: u32,
) -> (Vec<SdfPrimitive>, Vec<LeafAabb>, Vec<RaymarchPayload>) {
    let mut state = seed;
    let mut prims = Vec::with_capacity(n as usize);
    let mut leaves = Vec::with_capacity(n as usize);
    let mut payloads = Vec::with_capacity(n as usize);
    for i in 0..n {
        let pos = [lcg(&mut state) * 100.0, lcg(&mut state) * 100.0, lcg(&mut state) * 100.0];
        let radius = 0.5 + lcg(&mut state) * 0.5;
        let prim = SdfPrimitive {
            position: pos,
            type_tag: TYPE_SPHERE,
            rotation: Quat::IDENTITY.to_array(),
            scale: [1.0; 3],
            _pad0: 0.0,
            params: [radius, 0.0, 0.0, 0.0],
        };
        let aabb = primitive_aabb(&prim, 0.0);
        leaves.push(LeafAabb {
            aabb_min: aabb.min.to_array(),
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: aabb.max.to_array(),
            entity_id: i,
        });
        payloads.push(RaymarchPayload { smoothness: 0.0 });
        prims.push(prim);
    }
    (prims, leaves, payloads)
}

/// Convert a leaf-AABB list into the `(payload_id, Aabb)` pairs
/// `Bvh::build_gpu` consumes. Used by every test to bridge from the
/// scene generator into the BvhState API.
pub(super) fn items_from_leaves(leaves: &[LeafAabb]) -> Vec<(u32, ome_bvh::Aabb)> {
    leaves
        .iter()
        .enumerate()
        .map(|(i, l)| {
            (
                i as u32,
                ome_bvh::Aabb::new(
                    glam::Vec3::from_array(l.aabb_min),
                    glam::Vec3::from_array(l.aabb_max),
                ),
            )
        })
        .collect()
}

/// Sample-points payload used by the compute shader (vec4 padded to
/// keep std430 alignment simple).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct SamplePoint {
    pub(super) pos: [f32; 4],
}

pub(super) fn sample_points_grid(n: u32) -> Vec<SamplePoint> {
    // Deterministic grid of points across the same 100³ box used to
    // place the spheres. Enough samples land inside primitives'
    // AABBs that the per-role accumulator actually exercises a few
    // smooth_union / smooth_intersect calls per ray.
    let side = (n as f32).cbrt().ceil() as u32;
    let step = 100.0 / side as f32;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let x = (i % side) as f32 * step;
        let y = ((i / side) % side) as f32 * step;
        let z = (i / (side * side)) as f32 * step;
        out.push(SamplePoint { pos: [x, y, z, 0.0] });
    }
    out
}

/// Drive the in-flight build to completion, panicking if it fails to
/// resolve within a generous budget. PR-3's `BvhGpuBuild` resolves in
/// 1-2 iterations on a healthy queue.
pub(super) fn drive_bvh_to_completion(
    state: &mut BvhState,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    for _ in 0..16 {
        if let Some(outcome) = state.poll_swap(device, queue) {
            outcome.expect("BVH build must succeed for the test");
            return;
        }
        // Force progress on the queue. PollType::Wait without
        // a submission index waits on every outstanding submission.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(5)),
        });
    }
    panic!("BVH build did not resolve within 16 poll iterations");
}

/// Run the compute shader once and return the per-sample
/// distances. Re-uses the `BvhState`'s GPU-resident buffers —
/// matches the production binding layout. `entry_point` selects
/// between `cs_main` (BVH-driven) and `cs_fullscan` (brute-force
/// baseline used by the Lipschitz-bound test).
pub(super) fn run_eval_pass(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    state: &BvhState,
    primitives: &[SdfPrimitive],
    leaf_aabbs: &[LeafAabb],
    raymarch_payloads: &[RaymarchPayload],
    samples: &[SamplePoint],
    meta: &SceneMeta,
    entry_point: &str,
) -> Vec<f32> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("raymarch_bvh::test_compute"),
        source: wgpu::ShaderSource::Wgsl(TEST_COMPUTE_WGSL.into()),
    });

    let meta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_meta"),
        size: std::mem::size_of::<SceneMeta>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&meta_buffer, 0, bytemuck::bytes_of(meta));

    let prims_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_primitives"),
        size: (primitives.len() * std::mem::size_of::<SdfPrimitive>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&prims_buffer, 0, bytemuck::cast_slice(primitives));

    // Note: leaf_aabbs is uploaded INTO the BvhState's slot at
    // poll_swap time via queue.write_buffer; we re-upload here only
    // because the test wrapper needs a known-aligned binding. In
    // production the bind group already points at
    // bvh_state.current_leaf_aabbs() — same data either way.
    let leaves_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_leaf_aabbs"),
        size: (leaf_aabbs.len() * std::mem::size_of::<LeafAabb>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&leaves_buffer, 0, bytemuck::cast_slice(leaf_aabbs));

    let payloads_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_raymarch_payloads"),
        size: (raymarch_payloads.len() * std::mem::size_of::<RaymarchPayload>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&payloads_buffer, 0, bytemuck::cast_slice(raymarch_payloads));

    let samples_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_samples"),
        size: (samples.len() * std::mem::size_of::<SamplePoint>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&samples_buffer, 0, bytemuck::cast_slice(samples));

    let out_size = (samples.len() * 4) as u64;
    let out_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_out"),
        size: out_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test_bgl"),
        entries: &[
            bgl_entry(0, wgpu::BufferBindingType::Uniform),
            bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(3, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(4, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(5, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(6, wgpu::BufferBindingType::Storage { read_only: true }),
            bgl_entry(7, wgpu::BufferBindingType::Storage { read_only: false }),
        ],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test_bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: meta_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: prims_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: state.current_nodes().as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: state.current_sorted_indices().as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: leaves_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: payloads_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: samples_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: out_buffer.as_entire_binding() },
        ],
    });

    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("test_pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test_compute_pipeline"),
        layout: Some(&pl),
        module: &module,
        entry_point: Some(entry_point),
        compilation_options: Default::default(),
        cache: None,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test_compute_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("test_compute_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        let groups = (samples.len() as u32).div_ceil(64);
        pass.dispatch_workgroups(groups.max(1), 1, 1);
    }
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_staging"),
        size: out_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&out_buffer, 0, &staging, 0, out_size);
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).ok(); });
    device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(30)) })
        .expect("poll");
    rx.recv().expect("map_async sender").expect("map_async result");
    let data = slice.get_mapped_range();
    let v: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

fn bgl_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Default `SceneMeta` for tests that only exercise the ADD role and
/// skip the internal sky pass. Keeps each test's setup focused on
/// what it actually varies (samples, scene size, entry point).
pub(super) fn default_test_meta(state: &BvhState, primitive_count: usize) -> SceneMeta {
    SceneMeta {
        primitive_count: primitive_count as u32,
        bvh_n: state.current_n(),
        skip_internal_sky: 0,
        has_intersects: 0,
        has_subs: 0,
        k_int_scene: 0.0,
        k_sub_scene: 0.0,
        _pad0: 0,
        sky_top: [0.5, 0.7, 1.0, 1.0],
        sky_bottom: [0.1, 0.2, 0.4, 1.0],
    }
}
