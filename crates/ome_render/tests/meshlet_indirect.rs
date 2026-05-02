//! GPU integration tests: meshlet cull writes the indirect-draw args.
//!
//! Run with: `cargo test -p ome_render --test meshlet_indirect -- --test-threads=1`

mod common;

use common::{build_cube_mesh, read_u32, try_acquire_device};
use glam::{Mat4, Vec3};
use ome_render::meshlet::{build_default_meshlets, CullParams, DrawIndirectArgs, MeshletCull};

fn read_indirect_args(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
) -> DrawIndirectArgs {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet_indirect_args_staging"),
        size: std::mem::size_of::<DrawIndirectArgs>() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_indirect_args_readback"),
    });
    encoder.copy_buffer_to_buffer(
        buffer,
        0,
        &staging,
        0,
        std::mem::size_of::<DrawIndirectArgs>() as u64,
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
    let bytes = slice.get_mapped_range();
    *bytemuck::from_bytes::<DrawIndirectArgs>(&bytes)
}

#[test]
fn indirect_args_instance_count_matches_visible_meshlets_when_all_in_frustum() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let cull = MeshletCull::new(&device, gpu_mesh.meshlet_count.max(1) * 2, 124);

    let cam = Vec3::new(0.0, 0.0, 3.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let params = CullParams::new(proj * view, cam, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_test_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    let args = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    // Backface cone cull may drop meshlets whose normals all face away
    // from the camera (e.g. the +Z, +X, -X, +Y, -Y faces of a cube
    // viewed from -Z). Frustum-only would have kept them; that's
    // exactly the win we want from PR-5b.
    assert!(
        visible_count >= 1 && visible_count <= gpu_mesh.meshlet_count,
        "expected at least one front-facing meshlet visible: visible={visible_count} \
         total={total}",
        total = gpu_mesh.meshlet_count
    );
    assert_eq!(
        args.instance_count, visible_count,
        "indirect_args.instance_count must mirror the atomic visible_count"
    );
    assert_eq!(args.vertex_count, 124 * 3);
    assert_eq!(args.first_vertex, 0);
    assert_eq!(args.first_instance, 0);
}

#[test]
fn indirect_args_instance_count_is_zero_when_camera_faces_away() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let cull = MeshletCull::new(&device, gpu_mesh.meshlet_count.max(1) * 2, 124);

    let cam = Vec3::new(0.0, 0.0, 3.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, 100.0), Vec3::Y);
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 50.0);
    let params = CullParams::new(proj * view, cam, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_test_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let args = read_indirect_args(&device, &queue, cull.indirect_args_buffer());
    assert_eq!(
        args.instance_count, 0,
        "no meshlet should pass the frustum test when the camera looks the other way"
    );
}

#[test]
fn indirect_args_resets_between_dispatches() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let cull = MeshletCull::new(&device, gpu_mesh.meshlet_count.max(1) * 2, 124);

    let cam = Vec3::new(0.0, 0.0, 3.0);
    let visible_view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let visible_proj =
        Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let visible_params =
        CullParams::new(visible_proj * visible_view, cam, gpu_mesh.meshlet_count);

    let occluded_view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, 100.0), Vec3::Y);
    let occluded_params =
        CullParams::new(visible_proj * occluded_view, cam, gpu_mesh.meshlet_count);

    let mut encoder_a = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_frame_a"),
    });
    cull.dispatch(&device, &queue, &mut encoder_a, &gpu_mesh, &visible_params);
    queue.submit(std::iter::once(encoder_a.finish()));
    let args_a = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    let mut encoder_b = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_frame_b"),
    });
    cull.dispatch(&device, &queue, &mut encoder_b, &gpu_mesh, &occluded_params);
    queue.submit(std::iter::once(encoder_b.finish()));
    let args_b = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    assert!(args_a.instance_count >= 1);
    assert_eq!(args_b.instance_count, 0);
}
