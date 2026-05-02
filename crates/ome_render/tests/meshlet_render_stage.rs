//! End-to-end integration: ECS → MeshletRenderStage → readback.
//!
//! Phase 1.E.3a closure test. Spawns 2 `MeshRenderer` entities with
//! distinct `GlobalTransform`s, drives the stage manually (no plugin,
//! no editor), reads back the deferred color texture and asserts:
//!
//! - the stage ingests both entities (`stats.instances_uploaded == 2`)
//! - foreground pixels exist (some pixel is *not* the clear color)
//! - foreground pixels appear in BOTH the left and right halves of the
//!   target — a visual sanity check that the per-instance transforms
//!   actually drive different screen positions
//!
//! Run with:
//!   cargo test -p ome_render --test meshlet_render_stage -- --test-threads=1

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
    build_default_meshlets, key_from_handle, MeshletMesh, MeshletRenderStage,
    MeshletRenderStageConfig,
};

fn ecs_test_resources() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r
}

/// Reads a 2-D color texture (Rgba8Unorm, single mip, single layer)
/// back into a flat `Vec<u8>` of size `w * h * 4`. Pads `bytes_per_row`
/// to wgpu's 256-byte alignment requirement and strips the padding on
/// the CPU side so callers see a tightly packed buffer.
fn read_rgba8_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> (u32, u32, Vec<u8>) {
    let size = texture.size();
    let (w, h) = (size.width, size.height);
    let bpp = 4u32;
    let unpadded_row_bytes = w * bpp;
    let padded_row_bytes = unpadded_row_bytes.div_ceil(256) * 256;
    let total_padded = padded_row_bytes * h;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rgba8_readback_staging"),
        size: total_padded as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rgba8_readback_encoder"),
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
                bytes_per_row: Some(padded_row_bytes),
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
    let padded = slice.get_mapped_range().to_vec();

    let mut out = Vec::with_capacity((w * h * bpp) as usize);
    for row in 0..h {
        let start = (row * padded_row_bytes) as usize;
        let end = start + unpadded_row_bytes as usize;
        out.extend_from_slice(&padded[start..end]);
    }
    (w, h, out)
}

#[test]
fn render_stage_drives_two_ecs_entities_to_visible_pixels() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let cube = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&cube).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let mut resources = ecs_test_resources();
    let mut assets: Assets<MeshletMesh> = Assets::new();
    let asset_handle = assets.insert(meshlet_mesh.clone());
    resources.insert(assets);

    let config = MeshletRenderStageConfig {
        size: (256, 256),
        instance_capacity: 16,
        meshlet_capacity: 1024,
        materials: vec![
            MaterialParams::new([1.0, 0.4, 0.2, 1.0], 0.0, 0.5, 0.0),
            MaterialParams::new([0.2, 0.6, 1.0, 1.0], 0.0, 0.5, 0.0),
        ],
    };
    let mut stage = MeshletRenderStage::new(&device, config);
    stage
        .pipeline_mut()
        .register_mesh(asset_handle, &meshlet_mesh);

    let raw_key = key_from_handle(asset_handle);

    // Two entities — one to the left, one to the right of the origin.
    // Same registered mesh, distinct material ids so the deferred
    // shader picks different colours from `instances[]`.
    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            meshlet_mesh: Some(raw_key),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(-1.2, 0.0, 0.0)),
        });
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            meshlet_mesh: Some(raw_key),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(1.2, 0.0, 0.0)),
        });
    commands.apply(&mut resources);

    let cam_pos = Vec3::new(0.0, 0.5, 5.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);

    let stats = stage.render(
        &device,
        &queue,
        &resources,
        &gpu_mesh,
        proj * view,
        cam_pos,
    );
    assert_eq!(
        stats.instances_uploaded, 2,
        "stage should ingest 2 ECS entities"
    );
    assert!(
        stats.cull_threads >= 2,
        "scene cull thread budget should cover at least one meshlet per instance, got {}",
        stats.cull_threads
    );

    let (w, h, pixels) = read_rgba8_texture(&device, &queue, stage.color_texture());
    assert_eq!(pixels.len() as u32, w * h * 4);

    let mut foreground_total = 0usize;
    let mut foreground_left = 0usize;
    let mut foreground_right = 0usize;
    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 4) as usize;
            let r = pixels[idx];
            let g = pixels[idx + 1];
            let b = pixels[idx + 2];
            // Background pixels are the deferred clear (RGB=0, alpha=255).
            if r != 0 || g != 0 || b != 0 {
                foreground_total += 1;
                if x < w / 2 {
                    foreground_left += 1;
                } else {
                    foreground_right += 1;
                }
            }
        }
    }

    assert!(
        foreground_total > 0,
        "expected non-clear pixels in the deferred color buffer"
    );
    assert!(
        foreground_left > 0,
        "expected the left-positioned entity to produce visible pixels in the left half"
    );
    assert!(
        foreground_right > 0,
        "expected the right-positioned entity to produce visible pixels in the right half"
    );
}

#[test]
fn render_stage_with_no_entities_returns_zero_stats() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let cube = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&cube).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let mut resources = ecs_test_resources();
    let assets: Assets<MeshletMesh> = Assets::new();
    resources.insert(assets);

    let stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 8,
            meshlet_capacity: 256,
            materials: vec![MaterialParams::default()],
        },
    );

    let cam_pos = Vec3::new(0.0, 0.0, 5.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);

    let stats = stage.render(
        &device,
        &queue,
        &resources,
        &gpu_mesh,
        proj * view,
        cam_pos,
    );
    assert_eq!(stats.instances_uploaded, 0);
    assert_eq!(stats.cull_threads, 0);
}
