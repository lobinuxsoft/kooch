//! Integration: `UseMeshletPath` toggle drives `MeshletRenderStage` end
//! to end without a swapchain — Phase 1.E.3b smoke test.
//!
//! Cannot exercise [`RenderPlugin::build`] directly here because the
//! play-mode plugin needs a `wgpu::Surface` (swapchain), which a
//! headless test cannot acquire portably. Instead, the test reuses the
//! same code paths the plugin would: it constructs `MeshletRenderStage`
//! + `MeshletBlit` exactly as `init_renderers` does, mutates the
//! `UseMeshletPath` resource, and exercises the
//! `sync_assets_to_gpu` → `render_with_assets` → `blit` chain that the
//! plugin's render-frame system uses on the swapchain.
//!
//! The point: prove the orchestration plumbing is sound, even if the
//! present surface itself stays out of scope until the editor lands
//! 1.E.3c (multi-mesh) and 1.E.4 (visual confirmation).
//!
//! Run with:
//!   cargo test -p ome_render --test render_plugin_meshlet_toggle -- --test-threads=1

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use ome_core::assets::Assets;
use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::registry::ComponentRegistry;
use ome_ecs::hierarchy::global_transform::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::AccessTracker;
use ome_render::material::MaterialParams;
use ome_render::meshlet::{
    build_default_meshlets, key_from_handle, MeshletBlit, MeshletMesh, MeshletRenderStage,
    MeshletRenderStageConfig,
};
use ome_render::UseMeshletPath;

fn ecs_test_resources() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r
}

#[test]
fn toggle_default_is_disabled() {
    let toggle = UseMeshletPath::default();
    assert!(!toggle.enabled, "meshlet path must be opt-in until 1.E.4");
}

#[test]
fn render_with_assets_after_toggle_drives_full_chain() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // ECS world with two meshlet entities.
    let mut resources = ecs_test_resources();
    let mut assets: Assets<MeshletMesh> = Assets::new();
    let cube = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&cube).expect("build meshlets");
    let asset_handle = assets.insert(meshlet_mesh.clone());
    resources.insert(assets);
    resources.insert(UseMeshletPath { enabled: true });

    let raw_key = key_from_handle(asset_handle);
    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            meshlet_mesh: Some(raw_key),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(-0.8, 0.0, 0.0)),
        });
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            meshlet_mesh: Some(raw_key),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.8, 0.0, 0.0)),
        });
    commands.apply(&mut resources);

    // Same construction the plugin's `init_renderers` does.
    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (256, 256),
            instance_capacity: 16,
            meshlet_capacity: 1024,
            materials: vec![
                MaterialParams::new([1.0, 0.4, 0.2, 1.0], 0.0, 0.5, 0.0),
                MaterialParams::new([0.2, 0.6, 1.0, 1.0], 0.0, 0.5, 0.0),
            ],
        },
    );
    let target_format = wgpu::TextureFormat::Bgra8Unorm;
    let blit = MeshletBlit::new(&device, target_format);
    assert_eq!(blit.target_format(), target_format);

    // sync_assets_to_gpu is the plugin's bridge from `Assets<MeshletMesh>`
    // to GPU residency — without it, render_with_assets returns zero stats.
    assert_eq!(stage.gpu_mesh_count(), 0);
    stage.sync_assets_to_gpu(&device, &resources);
    assert_eq!(
        stage.gpu_mesh_count(),
        1,
        "sync should upload the cube mesh referenced by both ECS entities"
    );
    assert_eq!(stage.active_handle(), Some(asset_handle));

    let cam_pos = Vec3::new(0.0, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let stats = stage.render_with_assets(&device, &queue, &resources, proj * view, cam_pos);
    assert_eq!(
        stats.instances_uploaded, 2,
        "render_with_assets should ingest both ECS entities"
    );

    // Surface-style render attachment in the editor's typical format.
    let surface_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("plugin_test_surface_proxy"),
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let surface_view = surface_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("plugin_test_surface_encoder"),
    });
    // Clear pass first so a `LoadOp::Load` blit composes onto a defined
    // background — same order the plugin's render_passes uses.
    {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("plugin_test_surface_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    blit.blit(&device, &mut encoder, stage.color_view(), &surface_view);
    queue.submit(std::iter::once(encoder.finish()));

    // Read back the surface proxy and assert it carries non-clear pixels
    // — proves the chain stage→blit→surface composes.
    let bytes_per_pixel = 4u32;
    let unpadded_row = 256 * bytes_per_pixel;
    let padded_row = unpadded_row.div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("plugin_test_surface_staging"),
        size: (padded_row * 256) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &surface_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(256),
            },
        },
        wgpu::Extent3d {
            width: 256,
            height: 256,
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
    let bytes = slice.get_mapped_range().to_vec();

    let mut foreground = 0usize;
    for row in 0..256 {
        let start = row * padded_row as usize;
        for x in 0..256 {
            let idx = start + x * bytes_per_pixel as usize;
            // Bgra8Unorm: byte order is B, G, R, A. Background is all
            // zero from the clear pass; any non-zero channel = blit
            // wrote a meshlet pixel.
            if bytes[idx] != 0 || bytes[idx + 1] != 0 || bytes[idx + 2] != 0 {
                foreground += 1;
            }
        }
    }
    assert!(
        foreground > 0,
        "surface proxy should carry blit output (foreground pixels) after the meshlet path runs"
    );
}

#[test]
fn sync_assets_to_gpu_is_idempotent() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mut resources = ecs_test_resources();
    let mut assets: Assets<MeshletMesh> = Assets::new();
    let cube = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&cube).expect("build");
    let asset_handle = assets.insert(meshlet_mesh.clone());
    resources.insert(assets);

    let raw_key = key_from_handle(asset_handle);
    let mut commands = Commands::new();
    commands.spawn(&mut resources).insert(MeshRenderer {
        meshlet_mesh: Some(raw_key),
        visible: true,
        ..Default::default()
    }).insert(GlobalTransform::default());
    commands.apply(&mut resources);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            meshlet_capacity: 256,
            materials: vec![MaterialParams::default()],
        },
    );

    stage.sync_assets_to_gpu(&device, &resources);
    let count1 = stage.gpu_mesh_count();
    stage.sync_assets_to_gpu(&device, &resources);
    let count2 = stage.gpu_mesh_count();
    assert_eq!(count1, count2);
    assert_eq!(count1, 1);
}
