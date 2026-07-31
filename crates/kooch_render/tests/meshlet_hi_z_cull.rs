//! GPU integration test: Hi-Z occlusion test in `cs_cull_hi_z` rejects
//! meshlets whose bounding sphere lies behind the pyramid's depth.
//!
//! Uses an identity `view_proj` so the synthetic descriptor's
//! `bounds_center` is interpreted directly as an NDC point — keeps the
//! test readable, no perspective math needed.
//!
//! Hi-Z source is a 32×32 R32Float texture filled with a uniform
//! occluder depth (e.g. 0.5). Pyramid mips inherit the same value via
//! `cs_reduce_max`. Cull then rejects meshlets whose centre depth is
//! greater than the occluder.
//!
//! Run with:
//!   cargo test -p kooch_render --test meshlet_hi_z_cull

mod common;

use common::try_acquire_device;
use glam::{Mat4, Vec3};
use kooch_render::HiZ;
use kooch_render::mesh::{Aabb, MeshVertex};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, HiZTestParams, MeshletCull, MeshletDescriptor, MeshletMesh,
};

const HI_Z_DIM: u32 = 32;

fn synthetic_meshlet_at_ndc_depth(z: f32) -> MeshletMesh {
    // Single triangle; vertex content irrelevant for the cull test.
    let vertices = vec![
        MeshVertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        MeshVertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
        MeshVertex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        },
    ];
    let descriptor = MeshletDescriptor {
        vertex_offset: 0,
        triangle_offset: 0,
        vertex_count: 3,
        triangle_count: 1,
        aabb_min: [-0.05, -0.05, z - 0.05],
        parent_meshlet_index: kooch_render::meshlet::MESHLET_ROOT_PARENT,
        aabb_max: [0.05, 0.05, z + 0.05],
        lod_error: 0.0,
        // Bounds_center in "NDC space" because we pass view_proj = identity.
        bounds_center: [0.0, 0.0, z],
        bounding_radius: 0.05,
        cone_apex: [0.0, 0.0, z],
        cone_cutoff: 1.0, // disable cone cull (sentinel)
        cone_axis: [0.0, 0.0, 1.0],
        group_index: kooch_render::meshlet::MESHLET_GROUP_NONE,
        children_group_index: kooch_render::meshlet::MESHLET_GROUP_NONE,
        lod_level: 0,
        _pad4: 0,
        _pad5: 0,
    };
    MeshletMesh {
        vertices,
        meshlet_vertices: vec![0u32, 1, 2],
        meshlet_triangles: vec![0u8, 1, 2],
        meshlets: vec![descriptor],
        aabb: Aabb {
            min: Vec3::new(-0.05, -0.05, z - 0.05),
            max: Vec3::new(0.05, 0.05, z + 0.05),
        },
    }
}

fn upload_uniform_r32(device: &wgpu::Device, queue: &wgpu::Queue, value: f32) -> wgpu::Texture {
    let pixels = (HI_Z_DIM * HI_Z_DIM) as usize;
    let data = vec![value; pixels];
    let row_bytes = HI_Z_DIM * 4;

    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hi_z_cull_test_source"),
        size: wgpu::Extent3d {
            width: HI_Z_DIM,
            height: HI_Z_DIM,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice::<f32, u8>(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: Some(HI_Z_DIM),
        },
        wgpu::Extent3d {
            width: HI_Z_DIM,
            height: HI_Z_DIM,
            depth_or_array_layers: 1,
        },
    );
    tex
}

fn run_cull_with_hi_z(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mesh: &MeshletMesh,
    occluder_depth: f32,
) -> u32 {
    let gpu_mesh = mesh.upload(device);
    let cull = MeshletCull::new(device, 4, DEFAULT_MAX_TRIANGLES as u32);

    // Build Hi-Z from a synthetic uniform R32Float source.
    let src_tex = upload_uniform_r32(device, queue, occluder_depth);
    let src_view = src_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let hi_z = HiZ::new(device, HI_Z_DIM, HI_Z_DIM);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_cull_encoder"),
    });
    hi_z.build_from_r32(device, &mut encoder, &src_view);

    let cam = Vec3::ZERO;
    let cull_params = CullParams::new(Mat4::IDENTITY, cam, 1);
    let hi_z_params = HiZTestParams::new(Mat4::IDENTITY, HI_Z_DIM, HI_Z_DIM, hi_z.mip_count());
    cull.dispatch_with_hi_z(
        device,
        queue,
        &mut encoder,
        &gpu_mesh,
        &cull_params,
        &hi_z_params,
        hi_z.full_view(),
    );
    queue.submit(std::iter::once(encoder.finish()));

    common::read_u32(device, queue, cull.visible_count_buffer(), 0)
}

// All depth values below are in REVERSED-Z (#488): NDC depth 1 =
// near, 0 = far. Hi-Z occluder at depth d means "the closest
// fragment in the tile is at d"; a meshlet behind it has SMALLER
// ndc.z. The legacy `occluded_by_hi_z` test is now
// `sphere_nearest < tile_max` (max-reduced pyramid keeps closest
// fragment under reversed-Z).

#[test]
fn meshlet_behind_occluder_is_hi_z_culled() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Occluder Hi-Z at 0.5 (closest fragment in tile); meshlet at
    // 0.3 — farther than occluder under reversed-Z → culled.
    let mesh = synthetic_meshlet_at_ndc_depth(0.3);
    let visible = run_cull_with_hi_z(&device, &queue, &mesh, 0.5);
    assert_eq!(
        visible, 0,
        "meshlet behind (smaller ndc.z under reversed-Z) the occluder \
         should be rejected"
    );
}

#[test]
fn meshlet_in_front_of_occluder_is_kept() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Occluder Hi-Z at 0.5; meshlet at 0.7 — closer (larger ndc.z)
    // than the occluder under reversed-Z → visible.
    let mesh = synthetic_meshlet_at_ndc_depth(0.7);
    let visible = run_cull_with_hi_z(&device, &queue, &mesh, 0.5);
    assert_eq!(
        visible, 1,
        "meshlet in front (larger ndc.z under reversed-Z) of the \
         occluder should survive"
    );
}

#[test]
fn meshlet_at_far_plane_with_zero_occluder_is_culled() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Occluder Hi-Z saturated to 1.0 (= near plane in reversed-Z);
    // meshlet at 0.1 (far) → behind everything → culled.
    let mesh = synthetic_meshlet_at_ndc_depth(0.1);
    let visible = run_cull_with_hi_z(&device, &queue, &mesh, 1.0);
    assert_eq!(
        visible, 0,
        "occluder at near plane (1.0 in reversed-Z) culls everything farther"
    );
}

#[test]
fn empty_hi_z_one_does_not_cull_close_meshlets() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Hi-Z initialised to 0.0 (= far plane in reversed-Z, i.e. "no
    // closer geometry rendered yet"); any meshlet with ndc.z > 0
    // must survive.
    let mesh = synthetic_meshlet_at_ndc_depth(0.5);
    let visible = run_cull_with_hi_z(&device, &queue, &mesh, 0.0);
    assert_eq!(
        visible, 1,
        "Hi-Z initialised to far plane (0.0 in reversed-Z) should cull nothing"
    );
}
