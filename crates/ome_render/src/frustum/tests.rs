//! GPU correctness tests for [`FrustumCull`]. Compares the dispatched
//! shader's per-leaf `instance_count` output against a CPU brute-force
//! plane-AABB test over the same input. Skipped when no GPU adapter is
//! available — same policy as the BVH goldens.

use bytemuck::cast_slice;
use glam::{Vec3, Vec4};
use ome_bvh::{Aabb, IS_VISIBLE_MESH, LeafAabb, SharedBvhState};

use super::cull::{DrawIndexedIndirectArgs, FrustumCull, FrustumPlanes};

/// Headless GPU acquisition matching `ome_bvh::test_device::try_acquire`
/// and `raymarch::bvh::gpu_tests::harness::try_acquire_device`. Skipped
/// when no adapter has the timestamp features the BvhGpuBuilder needs.
fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let needs =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        if !adapter.features().contains(needs) {
            return None;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("frustum_cull::test_device"),
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

/// Pump the device until the kicked build resolves into the shared
/// state. Spins on `poll_swap` while submitting a `Wait` poll between
/// attempts so wgpu fires the `map_async` callbacks. Test-only.
fn drive_build_to_completion(
    shared: &mut SharedBvhState,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    loop {
        match shared.poll_swap(device, queue) {
            Some(Ok(_)) => return,
            Some(Err(e)) => panic!("SharedBvhState build failed: {e:?}"),
            None => {
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                });
            }
        }
    }
}

/// Build a (`items`, `leaf_aabbs`) pair where every leaf is flagged
/// `IS_VISIBLE_MESH` — broadphase-only flags would never reach the
/// frustum cull path. `entity_id == original_index` so the indirect
/// args buffer aligns 1:1 with the input ordering.
fn visible_mesh_scene(centres: &[Vec3], half: f32) -> (Vec<(u32, Aabb)>, Vec<LeafAabb>) {
    let items: Vec<(u32, Aabb)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, Aabb::from_centre(*c, Vec3::splat(half))))
        .collect();
    let leaves: Vec<LeafAabb> = items
        .iter()
        .map(|(i, a)| LeafAabb {
            aabb_min: a.min.to_array(),
            flags: IS_VISIBLE_MESH,
            aabb_max: a.max.to_array(),
            entity_id: *i,
        })
        .collect();
    (items, leaves)
}

/// CPU mirror of `aabb_in_frustum` from `frustum_cull.wgsl`. The shader
/// and this function MUST emit identical decisions for every AABB or
/// the byte-level test will fail.
fn cpu_aabb_in_frustum(aabb_min: [f32; 3], aabb_max: [f32; 3], planes: &[Vec4; 6]) -> bool {
    for plane in planes {
        let n = plane.truncate();
        let pv = Vec3::new(
            if n.x >= 0.0 { aabb_max[0] } else { aabb_min[0] },
            if n.y >= 0.0 { aabb_max[1] } else { aabb_min[1] },
            if n.z >= 0.0 { aabb_max[2] } else { aabb_min[2] },
        );
        if n.dot(pv) + plane.w < 0.0 {
            return false;
        }
    }
    true
}

/// Axis-aligned box frustum: inside iff `min <= p <= max`. Convenient
/// reference shape: every plane has a single non-zero normal component
/// so manual hand-calculations stay tractable.
fn axis_aligned_box_frustum(min: Vec3, max: Vec3) -> FrustumPlanes {
    FrustumPlanes([
        Vec4::new(1.0, 0.0, 0.0, -min.x), // x >= min.x  →  x - min.x >= 0
        Vec4::new(-1.0, 0.0, 0.0, max.x), // x <= max.x  → -x + max.x >= 0
        Vec4::new(0.0, 1.0, 0.0, -min.y),
        Vec4::new(0.0, -1.0, 0.0, max.y),
        Vec4::new(0.0, 0.0, 1.0, -min.z),
        Vec4::new(0.0, 0.0, -1.0, max.z),
    ])
}

/// Run the cull dispatch and read back the `n` indirect args.
fn dispatch_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &mut FrustumCull,
    shared: &SharedBvhState,
    planes: &FrustumPlanes,
    n: u32,
) -> Vec<DrawIndexedIndirectArgs> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frustum_cull_test_encoder"),
    });
    cull.cull(device, queue, &mut encoder, shared, planes, /* index_count */ 36);

    let bytes = n as u64 * std::mem::size_of::<DrawIndexedIndirectArgs>() as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frustum_cull_test_staging"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(cull.indirect_buffer(), 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv()
        .expect("staging map sender dropped")
        .expect("staging map failed");

    let data = staging.slice(..).get_mapped_range();
    let args: Vec<DrawIndexedIndirectArgs> =
        cast_slice::<u8, DrawIndexedIndirectArgs>(&data).to_vec();
    drop(data);
    staging.unmap();
    args
}

#[test]
fn frustum_cull_all_inside_marks_every_leaf_visible() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping all-inside test");
        return;
    };
    // 8 cubes inside a unit cluster, frustum covers a wide box.
    let centres: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let planes = axis_aligned_box_frustum(Vec3::splat(-100.0), Vec3::splat(100.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    assert_eq!(args.len(), 8);
    for (i, a) in args.iter().enumerate() {
        assert_eq!(a.instance_count, 1, "leaf {i} should be visible");
        assert_eq!(a.first_instance, i as u32);
        assert_eq!(a.index_count, 36);
    }
}

#[test]
fn frustum_cull_all_outside_marks_every_leaf_culled() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping all-outside test");
        return;
    };
    // 8 cubes far outside a tiny frustum at origin.
    let centres: Vec<Vec3> = (0..8)
        .map(|i| Vec3::new(1000.0 + i as f32, 0.0, 0.0))
        .collect();
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let planes = axis_aligned_box_frustum(Vec3::splat(-1.0), Vec3::splat(1.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    assert_eq!(args.len(), 8);
    for (i, a) in args.iter().enumerate() {
        assert_eq!(a.instance_count, 0, "leaf {i} should be culled");
    }
}

#[test]
fn frustum_cull_10k_cubes_matches_brute_force() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping 10k cubes test");
        return;
    };
    // 100x100 grid of unit cubes in the z=0 plane. Half-extent 0.4 so
    // the cubes are clearly separated; centres at integer coordinates.
    let mut centres = Vec::with_capacity(10_000);
    for j in 0..100 {
        for i in 0..100 {
            centres.push(Vec3::new(i as f32, j as f32, 0.0));
        }
    }
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let n = items.len() as u32;

    // Frustum: axis-aligned slice keeping x ≤ 49.5 and a fat y/z box.
    // The right plane bisects the grid between cube columns 49 and 50.
    let planes = axis_aligned_box_frustum(
        Vec3::new(-1000.0, -1000.0, -1000.0),
        Vec3::new(49.5, 1000.0, 1000.0),
    );

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves.clone(), /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, n);
    assert_eq!(args.len(), n as usize);

    // CPU brute force using the exact same plane-AABB algorithm as the
    // shader. Result must agree byte-perfect — the GPU cull is just
    // the same computation parallelised.
    let mut mismatches = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        let cpu_visible = (leaf.flags & IS_VISIBLE_MESH != 0)
            && cpu_aabb_in_frustum(leaf.aabb_min, leaf.aabb_max, &planes.0);
        let gpu_visible = args[i].instance_count == 1;
        if cpu_visible != gpu_visible {
            mismatches.push(i);
        }
    }
    assert!(
        mismatches.is_empty(),
        "CPU/GPU disagreement on {} leaves (first 10: {:?})",
        mismatches.len(),
        &mismatches[..mismatches.len().min(10)],
    );

    // Belt-and-suspenders: total visible count equals the obvious
    // closed-form (cubes with center.x ∈ {0..49} pass the right plane).
    let visible_count = args.iter().filter(|a| a.instance_count == 1).count();
    let expected = 50 * 100; // x ∈ {0..49} × y ∈ {0..99}
    assert_eq!(
        visible_count, expected,
        "expected {expected} visible cubes for the x ≤ 49.5 slice",
    );
}

#[test]
fn frustum_cull_skips_non_mesh_leaves() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull: no GPU adapter — skipping non-mesh-skip test");
        return;
    };
    // Mixed scene: 4 IS_VISIBLE_MESH cubes + 4 with IS_VISIBLE_MESH
    // cleared. The clear-flagged ones must come back culled regardless
    // of where they sit relative to the frustum.
    let centres: Vec<Vec3> = (0..8).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
    let items: Vec<(u32, Aabb)> = centres
        .iter()
        .enumerate()
        .map(|(i, c)| (i as u32, Aabb::from_centre(*c, Vec3::splat(0.4))))
        .collect();
    let leaves: Vec<LeafAabb> = items
        .iter()
        .enumerate()
        .map(|(i, (id, a))| LeafAabb {
            aabb_min: a.min.to_array(),
            // Even indices are IS_VISIBLE_MESH; odd indices have the
            // bit cleared (e.g. raymarch-only or pure colliders).
            flags: if i % 2 == 0 { IS_VISIBLE_MESH } else { 0 },
            aabb_max: a.max.to_array(),
            entity_id: *id,
        })
        .collect();
    let planes = axis_aligned_box_frustum(Vec3::splat(-100.0), Vec3::splat(100.0));

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, 8);
    for (i, a) in args.iter().enumerate() {
        let expect_visible = i % 2 == 0;
        assert_eq!(
            a.instance_count == 1,
            expect_visible,
            "leaf {i}: expected visible={expect_visible}, got instance_count={}",
            a.instance_count,
        );
    }
}
