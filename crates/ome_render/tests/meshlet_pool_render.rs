//! Multi-mesh acceptance: a scene with three distinct registered
//! meshes renders in a single cull dispatch via `cs_cull_scene_pool`
//! and produces foreground pixels for every instance.
//!
//! Validates the #446 + #457 migration end-to-end on the GPU.
//!
//! Run with:
//!   cargo test -p ome_render --test meshlet_pool_render

mod common;

use common::try_acquire_device;
use glam::{Mat4, Vec3};
use ome_core::Guid;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::registry::ComponentRegistry;
use ome_ecs::hierarchy::global_transform::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::AccessTracker;
use ome_core::resource::Resources;
use ome_render::material::{Material, MaterialPipeline};
use ome_render::mesh::{Mesh, MeshVertex};
use ome_render::meshlet::{
    build_default_meshlets, MeshletRenderStage, MeshletRenderStageConfig,
};

fn ecs_test_resources() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r
}

fn quad_mesh(scale: f32) -> Mesh {
    let v = |p: [f32; 3]| MeshVertex {
        position: p,
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    Mesh::from_arrays(
        vec![
            v([-scale, -scale, 0.0]),
            v([scale, -scale, 0.0]),
            v([scale, scale, 0.0]),
            v([-scale, scale, 0.0]),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn triangle_mesh(scale: f32) -> Mesh {
    let v = |p: [f32; 3]| MeshVertex {
        position: p,
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    Mesh::from_arrays(
        vec![
            v([-scale, -scale, 0.0]),
            v([scale, -scale, 0.0]),
            v([0.0, scale, 0.0]),
        ],
        vec![0, 1, 2],
    )
}

fn pentagon_mesh(scale: f32) -> Mesh {
    let v = |p: [f32; 3]| MeshVertex {
        position: p,
        normal: [0.0, 0.0, 1.0],
        uv: [0.0, 0.0],
    };
    let mut verts = Vec::with_capacity(6);
    verts.push(v([0.0, 0.0, 0.0]));
    for i in 0..5 {
        let a = (i as f32 / 5.0) * std::f32::consts::TAU;
        verts.push(v([scale * a.cos(), scale * a.sin(), 0.0]));
    }
    let mut idx = Vec::new();
    for i in 0..5 {
        let next = (i % 5) + 1;
        let after = (next % 5) + 1;
        idx.extend_from_slice(&[0, next as u32, after as u32]);
    }
    Mesh::from_arrays(verts, idx)
}

#[test]
fn three_distinct_meshes_render_in_single_cull_dispatch() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    // Three structurally different meshes registered into the pool.
    let mesh_a = build_default_meshlets(&quad_mesh(0.5)).expect("quad");
    let mesh_b = build_default_meshlets(&triangle_mesh(0.4)).expect("triangle");
    let mesh_c = build_default_meshlets(&pentagon_mesh(0.5)).expect("pentagon");

    let guid_a = Guid::new_v4();
    let guid_b = Guid::new_v4();
    let guid_c = Guid::new_v4();

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (256, 256),
            instance_capacity: 16,
            // Worst-case stride: mesh_a has the most meshlets after
            // build_default_meshlets, so meshlet_capacity must cover
            // instances * pool.max_meshlets_per_mesh.
            meshlet_capacity: 4096,
            ..Default::default()
        },
    );

    stage.ensure_gpu_mesh(&device, guid_a, &mesh_a);
    stage.ensure_gpu_mesh(&device, guid_b, &mesh_b);
    stage.ensure_gpu_mesh(&device, guid_c, &mesh_c);

    assert_eq!(
        stage.gpu_mesh_count(),
        3,
        "pool must hold all three registered meshes",
    );

    let mut resources = ecs_test_resources();
    let mut material_pipeline = MaterialPipeline::with_capacity(&device, 8);
    for mat in [
        Material::new([1.0, 0.4, 0.2, 1.0], 0.0, 0.5, 0.0),
        Material::new([0.2, 0.6, 1.0, 1.0], 0.0, 0.5, 0.0),
        Material::new([0.3, 1.0, 0.4, 1.0], 0.0, 0.5, 0.0),
    ] {
        material_pipeline.register(&queue, Guid::new_v4(), &mat);
    }
    resources.insert(material_pipeline);
    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid_a),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(-1.5, 0.0, 0.0)),
        });
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid_b),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)),
        });
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid_c),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0)),
        });
    commands.apply(&mut resources);

    let cam_pos = Vec3::new(0.0, 0.0, 5.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let proj = ome_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);

    let stats = stage.render_with_assets(&device, &queue, &resources, proj * view, cam_pos);
    assert_eq!(
        stats.instances_uploaded, 3,
        "stage must ingest all 3 entities (one per registered mesh)",
    );
    assert!(
        stats.cull_threads >= 3,
        "cull thread budget must cover at least one meshlet per instance, got {}",
        stats.cull_threads,
    );

    // Read the color attachment back. Three distinct meshes at
    // distinct world-space positions should produce foreground
    // coverage in three different horizontal bands.
    let (w, h, pixels) = read_rgba8(&device, &queue, stage.color_texture());
    assert_eq!(pixels.len() as u32, w * h * 4);

    let mut left = 0usize;
    let mut centre = 0usize;
    let mut right = 0usize;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let r = pixels[i];
            let g = pixels[i + 1];
            let b = pixels[i + 2];
            if r == 0 && g == 0 && b == 0 {
                continue;
            }
            // Three horizontal bands proportional to where each
            // entity's screen position lands. Approximate but
            // sufficient: a single cull dispatch must emit visible
            // meshlets across the whole horizontal extent.
            let third = w / 3;
            if x < third {
                left += 1;
            } else if x < 2 * third {
                centre += 1;
            } else {
                right += 1;
            }
        }
    }
    assert!(
        left > 0,
        "expected the left-positioned mesh to produce foreground pixels",
    );
    assert!(
        centre > 0,
        "expected the centre mesh to produce foreground pixels",
    );
    assert!(
        right > 0,
        "expected the right-positioned mesh to produce foreground pixels",
    );
}

fn read_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> (u32, u32, Vec<u8>) {
    let size = texture.size();
    let (w, h) = (size.width, size.height);
    let bpp = 4u32;
    let unpadded = w * bpp;
    let padded = unpadded.div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pool_render_staging"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("pool_render_readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
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
    let raw = slice.get_mapped_range().to_vec();
    let mut out = Vec::with_capacity((w * h * bpp) as usize);
    for row in 0..h {
        let s = (row * padded) as usize;
        let e = s + unpadded as usize;
        out.extend_from_slice(&raw[s..e]);
    }
    (w, h, out)
}
