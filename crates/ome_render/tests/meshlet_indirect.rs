//! GPU integration tests: meshlet cull writes the indirect-draw args.
//!
//! Run with: `cargo test -p ome_render --test meshlet_indirect -- --test-threads=1`
//!
//! The single-thread requirement is the global Mesa radv SIGSEGV
//! (documented in `project_phase1_progress.md` — `cargo test` parallel
//! workers crash inside Vulkan when several adapters init concurrently).
//! Lib-level tests are CPU-only and unaffected.
//!
//! Each test:
//! 1. Acquires a wgpu device (skips gracefully if none available).
//! 2. Builds a tiny meshlet mesh with known meshlet count.
//! 3. Runs the cull dispatcher with a CullParams either fully visible
//!    or fully occluded.
//! 4. Reads back the indirect-args buffer and asserts `instance_count`
//!    equals the expected visible count.

use glam::{Mat4, Vec3};
use ome_render::{
    mesh::{Mesh, MeshVertex},
    meshlet::{build_default_meshlets, CullParams, DrawIndirectArgs, MeshletCull},
};

fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet_indirect_test_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// Builds a 12-triangle mesh covering a small cube around the origin.
/// `meshopt::build_meshlets` clusters this into a handful of meshlets,
/// enough that frustum culling either keeps them all or drops them all.
fn build_cube_mesh() -> Mesh {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, // -Z
        4, 6, 5, 4, 7, 6, // +Z
        0, 4, 5, 0, 5, 1, // -Y
        3, 2, 6, 3, 6, 7, // +Y
        0, 3, 7, 0, 7, 4, // -X
        1, 5, 6, 1, 6, 2, // +X
    ];
    let vertices: Vec<MeshVertex> = positions
        .iter()
        .map(|p| MeshVertex {
            position: *p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        })
        .collect();
    Mesh::from_arrays(vertices, indices)
}

/// Reads four bytes back from `buffer` at byte `offset` as a u32.
fn read_u32(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64) -> u32 {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet_indirect_test_staging"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_indirect_test_readback"),
    });
    encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, 4);
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
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes);
    u32::from_le_bytes(buf)
}

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

    // Camera positioned far enough that every meshlet sits inside the
    // frustum. Wide FOV + look-at-origin guarantees inclusion.
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let params = CullParams::new(proj * view, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_test_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let visible_count = read_u32(&device, &queue, cull.visible_count_buffer(), 0);
    let args = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    assert_eq!(
        visible_count, gpu_mesh.meshlet_count,
        "every meshlet should pass the frustum test for an in-front camera"
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

    // Camera looks in the opposite direction — the cube ends up
    // entirely behind the near + far planes' normal, so every meshlet
    // is outside the frustum and `instance_count` settles at zero.
    let view = Mat4::look_at_rh(
        Vec3::new(0.0, 0.0, 3.0),
        Vec3::new(0.0, 0.0, 100.0),
        Vec3::Y,
    );
    let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.0, 0.1, 50.0);
    let params = CullParams::new(proj * view, gpu_mesh.meshlet_count);

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

    let visible_view =
        Mat4::look_at_rh(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y);
    let visible_proj =
        Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let visible_params =
        CullParams::new(visible_proj * visible_view, gpu_mesh.meshlet_count);

    let occluded_view = Mat4::look_at_rh(
        Vec3::new(0.0, 0.0, 3.0),
        Vec3::new(0.0, 0.0, 100.0),
        Vec3::Y,
    );
    let occluded_params =
        CullParams::new(visible_proj * occluded_view, gpu_mesh.meshlet_count);

    // Frame A — everything visible.
    let mut encoder_a = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_frame_a"),
    });
    cull.dispatch(&device, &queue, &mut encoder_a, &gpu_mesh, &visible_params);
    queue.submit(std::iter::once(encoder_a.finish()));
    let args_a = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    // Frame B — everything occluded.
    let mut encoder_b = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_cull_frame_b"),
    });
    cull.dispatch(&device, &queue, &mut encoder_b, &gpu_mesh, &occluded_params);
    queue.submit(std::iter::once(encoder_b.finish()));
    let args_b = read_indirect_args(&device, &queue, cull.indirect_args_buffer());

    assert!(args_a.instance_count >= 1);
    assert_eq!(args_b.instance_count, 0);
}

