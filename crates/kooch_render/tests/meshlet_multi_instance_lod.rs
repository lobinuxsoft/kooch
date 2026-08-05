//! GPU acceptance: 3 instances of the same dense LOD-chain mesh placed
//! at near / mid / far distances must pick distinct LOD bands.
//!
//! Validates the per-instance `group_max_err` slot decoding (#474).
//! Before the fix, `cs_lod_compute_group_max_err` keyed the atomic by
//! mesh-global `group_index`, so every instance of a mesh wrote into
//! the same slot range; pass 2 then read the closest instance's
//! verdict and every instance descended to LOD 0. With the fix each
//! instance owns a disjoint slot range (`inst.group_base` prefix sum)
//! and the selector picks LOD per instance independently — the far
//! instance must emit strictly fewer meshlets than the near one.
//!
//! Run with:
//!   cargo test -p kooch_render --test meshlet_multi_instance_lod

mod common;

use common::try_acquire_device;
use glam::{Mat4, Vec3};
use kooch_render::mesh::{Mesh, MeshVertex};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, GlobalMeshPool, LodConfig,
    MeshInstance, MeshletCull, MeshletCullPipelines, MeshletScene, SceneCullParams,
    build_meshlets_lod_chain, decode_scene_visible_id,
};
use std::collections::BTreeSet;

/// Curved grid in world `[-scale, scale]²` with a sinusoidal Z field.
/// Curvature is required so meshopt's simplifier produces a non-trivial
/// LOD chain — a flat grid collapses to near-zero error in one step
/// and the selector has nothing to pick between distances.
fn make_curved_grid(subdivisions: usize, scale: f32) -> Mesh {
    let n = subdivisions + 1;
    let mut verts = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let fx = (x as f32 / subdivisions as f32) * 2.0 - 1.0;
            let fy = (y as f32 / subdivisions as f32) * 2.0 - 1.0;
            let z = ((fx * 4.0).sin() + (fy * 4.0).cos()) * 0.25;
            verts.push(MeshVertex {
                position: [fx * scale, fy * scale, z * scale],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            });
        }
    }
    let mut idx = Vec::with_capacity(subdivisions * subdivisions * 6);
    for y in 0..subdivisions {
        for x in 0..subdivisions {
            let a = (y * n + x) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            idx.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    Mesh::from_arrays(verts, idx)
}

fn read_visible_meshlets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &MeshletCull,
    count: u32,
) -> Vec<u32> {
    if count == 0 {
        return Vec::new();
    }
    let bytes = (count as u64) * 4;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("multi_inst_lod_staging"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("multi_inst_lod_readback"),
    });
    enc.copy_buffer_to_buffer(cull.visible_meshlets_buffer(), 0, &staging, 0, bytes);
    queue.submit(std::iter::once(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    slice
        .get_mapped_range()
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn three_instances_pick_distinct_lod_bands() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    // Dense curved grid produces a multi-level LOD chain via
    // build_meshlets_lod_chain — a precondition for this test to be
    // meaningful (group_count > 0).
    let mesh = make_curved_grid(64, 5.0);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("build chain");

    let mut pool = GlobalMeshPool::new();
    let handle = pool.register(&chain);
    let mesh_desc = pool.mesh_descriptors[handle.mesh_id as usize];
    assert!(
        mesh_desc.group_count > 0,
        "test fixture must produce a non-trivial LOD chain (group_count > 0); \
         got 0 groups — bump grid subdivisions or LodConfig"
    );
    let meshlets_per_mesh = pool.max_meshlets_per_mesh();
    let gpu_pool = pool.upload(&device);

    // Three instances of the same mesh on -Z. Distances chosen so the
    // selector has clear signal: 5 m, 80 m, 1500 m. CPU prefix-sum
    // on group_base is the contract `MeshletPipeline::collect_scene_instances`
    // upholds in production; replicated here so the test exercises
    // the GPU path directly without standing up an ECS world.
    let mut instances: Vec<MeshInstance> = Vec::with_capacity(3);
    let positions = [-5.0_f32, -80.0, -1500.0];
    let mut running_base: u32 = 0;
    for pos_z in positions {
        let mut inst = MeshInstance::new(
            Mat4::from_translation(Vec3::new(0.0, 0.0, pos_z)),
            handle.mesh_id,
            0,
        );
        inst.group_base = running_base;
        running_base = running_base.saturating_add(mesh_desc.group_count);
        instances.push(inst);
    }
    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let total_threads = instances.len() as u32 * meshlets_per_mesh;
    let mut cull = MeshletCull::new(&device, total_threads * 2, DEFAULT_MAX_TRIANGLES as u32);
    let cull_pipelines = MeshletCullPipelines::new(&device);
    cull.ensure_group_capacity(&device, running_base.max(1));

    // Camera at origin looking down -Z with a 60 deg FOV. proj_scale_y
    // = 1 / tan(fovy/2) so the LOD selector receives the same
    // pixel-error factor the real renderer uses.
    let cam = Vec3::new(0.0, 0.0, 0.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, -1.0), Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 5000.0);
    let viewport_h = 1080.0_f32;
    let proj_scale_y = 1.0_f32 / (30.0_f32.to_radians()).tan();
    let cull_params = CullParams::new(proj * view, cam, meshlets_per_mesh).with_lod(
        viewport_h,
        proj_scale_y,
        1.0,
    );
    let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("multi_inst_lod_cull"),
    });
    cull.dispatch_scene_pool_atomic(
        &cull_pipelines,
        &device,
        &queue,
        &mut enc,
        &gpu_pool,
        &scene,
        &cull_params,
        &scene_params,
    );
    queue.submit(std::iter::once(enc.finish()));

    let visible_count = common::read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    assert!(
        visible_count > 0,
        "no meshlets emitted from any instance — cull regressed"
    );
    let visible = read_visible_meshlets(&device, &queue, &cull, visible_count);

    // Partition visible packed ids by instance.
    let mut sets: [BTreeSet<u32>; 3] = Default::default();
    for packed in &visible {
        let (inst, mid) = decode_scene_visible_id(*packed);
        assert!(
            inst < instances.len() as u32,
            "decoded instance_id {inst} out of range"
        );
        sets[inst as usize].insert(mid);
    }
    let near = sets[0].len();
    let mid_count = sets[1].len();
    let far = sets[2].len();

    eprintln!("multi-instance LOD counts: near={near} mid={mid_count} far={far}");

    assert!(
        near >= 1 && mid_count >= 1 && far >= 1,
        "every instance must emit at least one meshlet — got near={near} mid={mid_count} far={far}"
    );
    assert!(
        near >= mid_count,
        "near {near} must emit ≥ mid {mid_count} (LOD selector must be monotonic in distance)"
    );
    assert!(
        mid_count >= far,
        "mid {mid_count} must emit ≥ far {far} (LOD selector must be monotonic in distance)"
    );
    // The acceptance assertion for #474: per-instance independence.
    // If `near == far`, the atomic is still keyed globally and every
    // instance picks the closest one's LOD level — the original bug.
    assert!(
        near > far,
        "per-instance LOD must produce distinct sets — near {near} should strictly exceed far {far}; \
         equality means group_max_err is still keyed mesh-globally and the fix regressed"
    );
}
