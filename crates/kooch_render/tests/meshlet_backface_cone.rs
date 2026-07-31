//! GPU integration test: backface cone cull rejects meshlets whose
//! normal cone points away from the camera.
//!
//! Builds a synthetic [`MeshletMesh`] with a single descriptor whose
//! cone parameters are controlled by hand, so the test does not depend
//! on `meshopt`'s clustering heuristics for the assertion.
//!
//! Run with:
//!   cargo test -p kooch_render --test meshlet_backface_cone

mod common;

use common::try_acquire_device;
use glam::{Mat4, Vec3};
use kooch_render::mesh::{Aabb, MeshVertex};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, MeshletCull, MeshletDescriptor, MeshletMesh,
};

fn synthetic_meshlet_facing(normal: Vec3) -> MeshletMesh {
    // Single triangle at the origin — vertex data only matters insofar
    // as the cull shader needs a non-empty buffer to bind. The cone
    // parameters in the descriptor are what drive the test.
    let vertices = vec![
        MeshVertex {
            position: [0.0, 0.0, 0.0],
            normal: normal.to_array(),
            uv: [0.0, 0.0],
        },
        MeshVertex {
            position: [1.0, 0.0, 0.0],
            normal: normal.to_array(),
            uv: [0.0, 0.0],
        },
        MeshVertex {
            position: [0.0, 1.0, 0.0],
            normal: normal.to_array(),
            uv: [0.0, 0.0],
        },
    ];
    let meshlet_vertices = vec![0u32, 1, 2];
    let meshlet_triangles = vec![0u8, 1, 2];

    // meshopt convention: `cone_axis` points along the meshlet's
    // average front-face normal. For a triangle whose normal is
    // `normal`, the cone axis IS `normal`. A camera sitting on the
    // `-normal` side of the apex sees the meshlet from behind and
    // gets culled by the shader's backface test.
    let cone_axis = normal.normalize();
    let descriptor = MeshletDescriptor {
        vertex_offset: 0,
        triangle_offset: 0,
        vertex_count: 3,
        triangle_count: 1,
        aabb_min: [0.0, 0.0, 0.0],
        parent_meshlet_index: kooch_render::meshlet::MESHLET_ROOT_PARENT,
        aabb_max: [1.0, 1.0, 0.0],
        lod_error: 0.0,
        bounds_center: [0.5, 0.5, 0.0],
        bounding_radius: 1.0,
        cone_apex: [0.5, 0.5, 0.0],
        // Tight cone (cutoff close to 1.0) so the test is precise: the
        // camera must be very nearly along `cone_axis` for the meshlet
        // to be culled.
        cone_cutoff: 0.7,
        cone_axis: cone_axis.to_array(),
        group_index: kooch_render::meshlet::MESHLET_GROUP_NONE,
        children_group_index: kooch_render::meshlet::MESHLET_GROUP_NONE,
        lod_level: 0,
        _pad4: 0,
        _pad5: 0,
    };

    MeshletMesh {
        vertices,
        meshlet_vertices,
        meshlet_triangles,
        meshlets: vec![descriptor],
        aabb: Aabb {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 1.0, 0.0),
        },
    }
}

fn read_visible_count(device: &wgpu::Device, queue: &wgpu::Queue, cull: &MeshletCull) -> u32 {
    common::read_u32(device, queue, cull.visible_count_buffer(), 0)
}

#[test]
fn meshlet_facing_camera_passes_cone_cull() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Triangle facing +Z; camera placed in front (+Z).
    let mesh = synthetic_meshlet_facing(Vec3::Z);
    let gpu_mesh = mesh.upload(&device);

    let cull = MeshletCull::new(&device, 4, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.5, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.5, 0.5, 0.0), Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let params = CullParams::new(proj * view, cam, 1);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("backface_test_front_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let visible = read_visible_count(&device, &queue, &cull);
    assert_eq!(visible, 1, "front-facing meshlet should survive cone cull");
}

#[test]
fn meshlet_facing_away_is_culled_by_cone() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Triangle still facing +Z (so cone_axis is also +Z, meshopt
    // convention). Camera placed behind (-Z): the camera-to-apex
    // vector aligns with cone_axis → backface cull triggers.
    let mesh = synthetic_meshlet_facing(Vec3::Z);
    let gpu_mesh = mesh.upload(&device);

    let cull = MeshletCull::new(&device, 4, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.5, 0.5, -5.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.5, 0.5, 0.0), Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let params = CullParams::new(proj * view, cam, 1);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("backface_test_back_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let visible = read_visible_count(&device, &queue, &cull);
    assert_eq!(
        visible, 0,
        "back-facing meshlet should be rejected by the cone test (camera lies along cone_axis)"
    );
}

#[test]
fn cone_cutoff_one_disables_cull_even_when_camera_aligned() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Same setup as the cull-positive case, but cone_cutoff == 1.0
    // marks the meshlet as 'normals divergent — do not cone-cull'.
    // The shader honours the sentinel and the meshlet survives.
    let mut mesh = synthetic_meshlet_facing(Vec3::Z);
    mesh.meshlets[0].cone_cutoff = 1.0;
    let gpu_mesh = mesh.upload(&device);

    let cull = MeshletCull::new(&device, 4, DEFAULT_MAX_TRIANGLES as u32);

    let cam = Vec3::new(0.5, 0.5, -5.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.5, 0.5, 0.0), Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let params = CullParams::new(proj * view, cam, 1);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("backface_test_sentinel_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &params);
    queue.submit(std::iter::once(encoder.finish()));

    let visible = read_visible_count(&device, &queue, &cull);
    assert_eq!(
        visible, 1,
        "cone_cutoff sentinel (1.0) must short-circuit the cone test"
    );
}
