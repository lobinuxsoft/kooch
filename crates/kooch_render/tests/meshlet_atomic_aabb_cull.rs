//! GPU integration test: scene-pool atomic cull (#454.4) frustum
//! rejection via AABB-vs-frustum (`aabb_outside_frustum_local`).
//!
//! Exercises `dispatch_scene_pool_atomic` directly so the AABB switch
//! introduced in #454.4 can be verified end-to-end on a real adapter
//! — the `meshlet_scene_cull` suite covers the legacy `cs_cull_scene`
//! entry which still uses `sphere_outside_frustum` and therefore would
//! pass even if the AABB port were broken.
//!
//! Asserts:
//!   - instances clearly inside the frustum survive cull
//!   - instances clearly outside the frustum (behind camera, far off
//!     to the side) are dropped from `visible_meshlets`

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, GlobalMeshPool, MeshInstance, MeshletCull,
    MeshletCullPipelines, MeshletScene, SceneCullParams, build_default_meshlets,
    decode_scene_visible_id,
};
use std::collections::BTreeSet;

#[test]
fn atomic_pool_cull_drops_off_frustum_aabb() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");

    // Build a single-mesh pool the atomic path can dispatch over.
    let mut pool = GlobalMeshPool::new();
    let mesh_handle = pool.register(&meshlet_mesh);
    let gpu_pool = pool.upload(&device);
    let meshlets_per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);

    // 4 instances: 0/1 in front of cam, 2 behind camera, 3 way to
    // the right outside a 45° FOV. The AABB version should drop the
    // last two; sphere bounds would also drop them, but we keep the
    // gap large so even a buggy AABB couldn't pass them.
    let instances = vec![
        MeshInstance::new(
            Mat4::from_translation(Vec3::new(-0.5, 0.0, 0.0)),
            mesh_handle.mesh_id,
            0,
        ),
        MeshInstance::new(
            Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)),
            mesh_handle.mesh_id,
            0,
        ),
        MeshInstance::new(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -200.0)),
            mesh_handle.mesh_id,
            0,
        ),
        MeshInstance::new(
            Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0)),
            mesh_handle.mesh_id,
            0,
        ),
    ];

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let total_threads = instances.len() as u32 * meshlets_per_mesh;
    let mut cull = MeshletCull::new(&device, total_threads * 2, DEFAULT_MAX_TRIANGLES as u32);
    let cull_pipelines = MeshletCullPipelines::new(&device);
    cull.ensure_capacity(&device, total_threads);
    cull.ensure_group_capacity(&device, total_threads);

    let cam = Vec3::new(0.0, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(45.0_f32.to_radians(), 1.0, 0.1, 50.0);
    let view_proj = proj * view;
    let cull_params = CullParams::new(view_proj, cam, meshlets_per_mesh);
    let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("atomic_pool_cull_aabb_encoder"),
    });
    cull.dispatch_scene_pool_atomic(
        &cull_pipelines,
        &device,
        &queue,
        &mut encoder,
        &gpu_pool,
        &scene,
        &cull_params,
        &scene_params,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = common::read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    assert!(
        visible_count > 0,
        "in-frustum instances should produce at least one visible meshlet"
    );

    let visible: Vec<u32> = {
        let byte_count = (visible_count as u64) * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atomic_pool_cull_visible_staging"),
            size: byte_count,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("atomic_pool_cull_readback"),
        });
        enc.copy_buffer_to_buffer(cull.visible_meshlets_buffer(), 0, &staging, 0, byte_count);
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
    };

    let mut unique_instances = BTreeSet::new();
    for packed in &visible {
        let (instance_id, _) = decode_scene_visible_id(*packed);
        unique_instances.insert(instance_id);
    }

    assert!(
        unique_instances.contains(&0),
        "instance 0 (in frustum) should survive AABB cull, got {unique_instances:?}",
    );
    assert!(
        unique_instances.contains(&1),
        "instance 1 (in frustum) should survive AABB cull, got {unique_instances:?}",
    );
    assert!(
        !unique_instances.contains(&2),
        "instance 2 (z=-200, behind camera) must be AABB-rejected",
    );
    assert!(
        !unique_instances.contains(&3),
        "instance 3 (x=100, outside 45° FOV) must be AABB-rejected",
    );
}
