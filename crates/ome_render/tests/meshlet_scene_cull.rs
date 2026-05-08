//! GPU integration test: scene-wide cull dispatch (Phase 1.E.1).
//!
//! Uploads 4 cube instances at distinct world positions, runs ONE
//! dispatch of `cs_cull_scene` over the full (instance × meshlet)
//! grid, and asserts:
//!   - frustum-visible instances survive
//!   - off-screen instances get culled
//!   - the packed `(instance_id, meshlet_id)` decoding matches the
//!     CPU mirror in `meshlet::scene::decode_scene_visible_id`
//!
//! Run with:
//!   cargo test -p ome_render --test meshlet_scene_cull

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use ome_render::meshlet::{
    build_default_meshlets, decode_scene_visible_id, CullParams, MeshInstance, MeshletCull,
    MeshletScene, SceneCullParams, DEFAULT_MAX_TRIANGLES,
};
use std::collections::BTreeSet;

fn read_visible_meshlets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cull: &MeshletCull,
    count: u32,
) -> Vec<u32> {
    if count == 0 {
        return Vec::new();
    }
    let byte_count = (count as u64) * 4;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene_cull_visible_staging"),
        size: byte_count,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene_cull_readback"),
    });
    encoder.copy_buffer_to_buffer(
        cull.visible_meshlets_buffer(),
        0,
        &staging,
        0,
        byte_count,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().to_vec();
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn scene_cull_visits_each_instance_once_when_all_in_frustum() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);
    let meshlets_per_mesh = gpu_mesh.meshlet_count;

    // 4 instances, all in front of the camera, spaced along X.
    let instances = vec![
        MeshInstance::new(Mat4::from_translation(Vec3::new(-1.5, 0.0, 0.0)), 0, 0),
        MeshInstance::new(Mat4::from_translation(Vec3::new(-0.5, 0.0, 0.0)), 0, 0),
        MeshInstance::new(Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)), 0, 0),
        MeshInstance::new(Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0)), 0, 0),
    ];

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let total_threads = instances.len() as u32 * meshlets_per_mesh;
    let cull = MeshletCull::new(&device, total_threads * 2, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.0, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
    let proj = ome_render::perspective_rh_reverse_z(80.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let cull_params = CullParams::new(proj * view, cam, meshlets_per_mesh);
    let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene_cull_all_visible_encoder"),
    });
    cull.dispatch_scene(
        &device,
        &queue,
        &mut encoder,
        &gpu_mesh,
        &scene,
        &cull_params,
        &scene_params,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = common::read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    assert!(
        visible_count > 0,
        "at least one (instance, meshlet) pair should survive"
    );

    let visible = read_visible_meshlets(&device, &queue, &cull, visible_count);
    let mut unique_instances = BTreeSet::new();
    for packed in &visible {
        let (instance_id, meshlet_id) = decode_scene_visible_id(*packed);
        assert!(
            instance_id < instances.len() as u32,
            "decoded instance_id {instance_id} out of range"
        );
        assert!(
            meshlet_id < meshlets_per_mesh,
            "decoded meshlet_id {meshlet_id} >= meshlets_per_mesh {meshlets_per_mesh}"
        );
        unique_instances.insert(instance_id);
    }
    assert_eq!(
        unique_instances.len(),
        instances.len(),
        "every instance should contribute at least one visible meshlet (got {unique_instances:?})"
    );
}

#[test]
fn scene_cull_drops_off_screen_instances() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);
    let meshlets_per_mesh = gpu_mesh.meshlet_count;

    // 4 instances; only the first two are inside the frustum, the
    // last two are far enough that the cube AABB clearly leaves it.
    let instances = vec![
        MeshInstance::new(Mat4::from_translation(Vec3::new(-0.5, 0.0, 0.0)), 0, 0),
        MeshInstance::new(Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)), 0, 0),
        // instance 2: behind camera + far along -Z
        MeshInstance::new(Mat4::from_translation(Vec3::new(0.0, 0.0, -200.0)), 0, 0),
        // instance 3: way off to the right, outside the FOV
        MeshInstance::new(Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0)), 0, 0),
    ];

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let total_threads = instances.len() as u32 * meshlets_per_mesh;
    let cull = MeshletCull::new(&device, total_threads * 2, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.0, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
    // Narrow FOV so instance 3 is well outside.
    let proj = ome_render::perspective_rh_reverse_z(45.0_f32.to_radians(), 1.0, 0.1, 50.0);
    let cull_params = CullParams::new(proj * view, cam, meshlets_per_mesh);
    let scene_params = SceneCullParams::new(instances.len() as u32, meshlets_per_mesh);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene_cull_off_screen_encoder"),
    });
    cull.dispatch_scene(
        &device,
        &queue,
        &mut encoder,
        &gpu_mesh,
        &scene,
        &cull_params,
        &scene_params,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = common::read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    let visible = read_visible_meshlets(&device, &queue, &cull, visible_count);

    let mut unique_instances = BTreeSet::new();
    for packed in &visible {
        let (instance_id, _) = decode_scene_visible_id(*packed);
        unique_instances.insert(instance_id);
    }
    // Instances 2 and 3 must be culled — they are outside the frustum.
    assert!(
        !unique_instances.contains(&2),
        "instance 2 (behind camera, z=-200) should be culled"
    );
    assert!(
        !unique_instances.contains(&3),
        "instance 3 (off-screen, x=100) should be culled"
    );
    // Instances 0 and 1 must remain visible.
    assert!(
        unique_instances.contains(&0),
        "instance 0 should survive frustum cull"
    );
    assert!(
        unique_instances.contains(&1),
        "instance 1 should survive frustum cull"
    );
}

#[test]
fn scene_cull_with_zero_instances_is_no_op() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);
    let meshlets_per_mesh = gpu_mesh.meshlet_count;

    // Capacity 4 but no upload — instance buffer is uninitialised,
    // and we tell the shader instance_count = 0 so it never reads.
    let scene = MeshletScene::new(&device, 4);

    let cull = MeshletCull::new(&device, 256, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.0, 0.0, 5.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = ome_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let cull_params = CullParams::new(proj * view, cam, meshlets_per_mesh);
    let scene_params = SceneCullParams::new(0, meshlets_per_mesh);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene_cull_zero_instances_encoder"),
    });
    cull.dispatch_scene(
        &device,
        &queue,
        &mut encoder,
        &gpu_mesh,
        &scene,
        &cull_params,
        &scene_params,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = common::read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    assert_eq!(
        visible_count, 0,
        "zero instances should produce zero visible meshlets"
    );
}
