//! End-to-end GPU integration test: cull → indirect draw → pixels.
//!
//! Builds a small cube, runs the meshlet pipeline (compute cull
//! followed by single `draw_indirect`), reads back the color
//! attachment, and asserts the rasterizer painted something visible
//! (i.e. at least some non-clear pixels carry a recognisable normal-
//! debug color).
//!
//! Run with: `cargo test -p kooch_render --test meshlet_render`

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, MeshletCull, MeshletDrawer, build_default_meshlets,
    meshlet_bind_group, meshlet_bind_group_layout,
};

const RT_WIDTH: u32 = 64;
const RT_HEIGHT: u32 = 64;
// 64 px * 4 bytes/px = 256, exactly wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`.
const RT_BYTES_PER_ROW: u32 = 64 * 4;
const RT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
fn meshlet_pipeline_renders_visible_cube_pixels() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // ------------------------------------------------------------------
    // Mesh + GPU upload
    // ------------------------------------------------------------------
    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    // ------------------------------------------------------------------
    // Pipeline assembly
    // ------------------------------------------------------------------
    let cull = MeshletCull::new(
        &device,
        gpu_mesh.meshlet_count.max(1) * 2,
        DEFAULT_MAX_TRIANGLES as u32,
    );
    let drawer = MeshletDrawer::new(
        &device,
        RT_FORMAT,
        Some(DEPTH_FORMAT),
        cull.meshlet_bind_group_layout(),
        None,
    );
    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    // ------------------------------------------------------------------
    // Render targets
    // ------------------------------------------------------------------
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshlet_render_color"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: RT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshlet_render_depth"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // ------------------------------------------------------------------
    // Camera in front of the cube
    // ------------------------------------------------------------------
    let cam = Vec3::new(0.0, 0.0, 2.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(
        60.0_f32.to_radians(),
        RT_WIDTH as f32 / RT_HEIGHT as f32,
        0.1,
        100.0,
    );
    let view_proj = proj * view;
    let model = Mat4::IDENTITY;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    // ------------------------------------------------------------------
    // Encode + submit
    // ------------------------------------------------------------------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_render_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &cull_params);
    drawer.render(
        &device,
        &queue,
        &mut encoder,
        &color_view,
        Some(&depth_view),
        &meshlet_bg,
        &cull,
        view_proj,
        model,
        Some(wgpu::Color::BLACK),
    );

    // Copy color RT → staging buffer for readback.
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet_render_color_staging"),
        size: (RT_BYTES_PER_ROW * RT_HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(RT_BYTES_PER_ROW),
                rows_per_image: Some(RT_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    // ------------------------------------------------------------------
    // Readback + assertions
    // ------------------------------------------------------------------
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().to_vec();

    let mut non_clear_pixels = 0usize;
    let mut max_brightness = 0u8;
    for y in 0..RT_HEIGHT {
        for x in 0..RT_WIDTH {
            let idx = (y * RT_BYTES_PER_ROW + x * 4) as usize;
            let r = bytes[idx];
            let g = bytes[idx + 1];
            let b = bytes[idx + 2];
            if r != 0 || g != 0 || b != 0 {
                non_clear_pixels += 1;
            }
            max_brightness = max_brightness.max(r.max(g).max(b));
        }
    }

    let total_pixels = (RT_WIDTH * RT_HEIGHT) as usize;
    assert!(
        non_clear_pixels > total_pixels / 100,
        "expected the rasterizer to paint at least 1% of the frame; got \
         {non_clear_pixels}/{total_pixels} non-clear pixels (max brightness {max_brightness})"
    );
    assert!(
        max_brightness > 64,
        "expected at least one well-lit pixel; max channel value was {max_brightness}"
    );
}

#[test]
fn meshlet_pipeline_renders_nothing_when_camera_faces_away() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let cull = MeshletCull::new(
        &device,
        gpu_mesh.meshlet_count.max(1) * 2,
        DEFAULT_MAX_TRIANGLES as u32,
    );
    let drawer = MeshletDrawer::new(
        &device,
        RT_FORMAT,
        Some(DEPTH_FORMAT),
        cull.meshlet_bind_group_layout(),
        None,
    );
    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshlet_render_color_empty"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: RT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("meshlet_render_depth_empty"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    // Camera looking the other way — every meshlet is culled, the
    // indirect draw runs with `instance_count = 0` and the rasterizer
    // emits exactly the clear color.
    let cam = Vec3::new(0.0, 0.0, 2.0);
    let view = Mat4::look_at_rh(cam, Vec3::new(0.0, 0.0, 100.0), Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(45.0_f32.to_radians(), 1.0, 0.1, 50.0);
    let view_proj = proj * view;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("meshlet_render_encoder_empty"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &cull_params);
    drawer.render(
        &device,
        &queue,
        &mut encoder,
        &color_view,
        Some(&depth_view),
        &meshlet_bg,
        &cull,
        view_proj,
        Mat4::IDENTITY,
        Some(wgpu::Color::BLACK),
    );

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet_render_empty_staging"),
        size: (RT_BYTES_PER_ROW * RT_HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(RT_BYTES_PER_ROW),
                rows_per_image: Some(RT_HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
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

    let mut non_clear_pixels = 0usize;
    for y in 0..RT_HEIGHT {
        for x in 0..RT_WIDTH {
            let idx = (y * RT_BYTES_PER_ROW + x * 4) as usize;
            if bytes[idx] != 0 || bytes[idx + 1] != 0 || bytes[idx + 2] != 0 {
                non_clear_pixels += 1;
            }
        }
    }

    assert_eq!(
        non_clear_pixels, 0,
        "rasterizer should have skipped every pixel when no meshlets were visible"
    );
}
