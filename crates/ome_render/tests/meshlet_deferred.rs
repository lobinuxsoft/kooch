//! End-to-end GPU integration: meshlet visibility-buffer + deferred
//! shading renders the cube test scene with non-clear pixels.
//!
//! Same camera + cube setup as `meshlet_render.rs`, but the pipeline
//! goes:
//!   cull → vbuf rasterize (R32Uint + depth) → deferred shade (compute
//!   into Rgba8Unorm storage texture).
//!
//! Run with:
//!   cargo test -p ome_render --test meshlet_deferred -- --test-threads=1

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use ome_render::material::{MaterialParams, MaterialPool};
use ome_render::meshlet::{
    build_default_meshlets, meshlet_bind_group, meshlet_bind_group_layout, CullParams,
    MeshletCull, MeshletDeferredShader, MeshletVisRasterizer, DEFAULT_MAX_TRIANGLES,
    DEFERRED_COLOR_FORMAT, VISIBILITY_BUFFER_FORMAT,
};

const RT_WIDTH: u32 = 64;
const RT_HEIGHT: u32 = 64;
const ROW_BYTES: u32 = 64 * 4;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
fn vis_buffer_plus_deferred_paints_visible_cube_pixels() {
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
    let vbuf_raster = MeshletVisRasterizer::new(
        &device,
        Some(DEPTH_FORMAT),
        cull.meshlet_bind_group_layout(),
        None,
    );
    let deferred = MeshletDeferredShader::new(&device, cull.meshlet_bind_group_layout());

    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    // Targets
    let vbuf_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vbuf_test"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VISIBILITY_BUFFER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let vbuf_view = vbuf_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deferred_color"),
        size: wgpu::Extent3d {
            width: RT_WIDTH,
            height: RT_HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEFERRED_COLOR_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vbuf_depth"),
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

    let cam = Vec3::new(0.0, 0.0, 2.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = ome_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view_proj = proj * view;
    let model = Mat4::IDENTITY;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("vbuf_deferred_encoder"),
    });
    cull.dispatch(&device, &queue, &mut encoder, &gpu_mesh, &cull_params);
    vbuf_raster.render(
        &device,
        &queue,
        &mut encoder,
        &vbuf_view,
        Some(&depth_view),
        &meshlet_bg,
        &cull,
        view_proj,
        model,
        0,
    );
    let materials = MaterialPool::new(
        &device,
        &[MaterialParams::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.0)],
    );
    let material_bg = materials.bind_group(&device);
    deferred.shade(
        &device,
        &queue,
        &mut encoder,
        &vbuf_view,
        &color_view,
        &meshlet_bg,
        &material_bg,
        view_proj,
        model,
        (RT_WIDTH, RT_HEIGHT),
        0,
    );

    // Readback color
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("deferred_color_staging"),
        size: (ROW_BYTES * RT_HEIGHT) as u64,
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
                bytes_per_row: Some(ROW_BYTES),
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

    let mut non_clear = 0usize;
    let mut max_channel = 0u8;
    for y in 0..RT_HEIGHT {
        for x in 0..RT_WIDTH {
            let off = (y * ROW_BYTES + x * 4) as usize;
            let r = bytes[off];
            let g = bytes[off + 1];
            let b = bytes[off + 2];
            if r != 0 || g != 0 || b != 0 {
                non_clear += 1;
            }
            max_channel = max_channel.max(r.max(g).max(b));
        }
    }
    let total = (RT_WIDTH * RT_HEIGHT) as usize;
    assert!(
        non_clear > total / 100,
        "vbuf+deferred should paint > 1% of the frame; got {non_clear}/{total}"
    );
    assert!(
        max_channel > 64,
        "expected at least one well-lit pixel; max channel was {max_channel}"
    );
}
