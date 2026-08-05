//! GPU integration test: deferred shading modulates by material id.
//!
//! Renders the same cube + camera setup with two different material
//! ids and asserts the resulting colors differ.
//!
//! Run with:
//!   cargo test -p kooch_render --test meshlet_materials

mod common;

use common::{build_cube_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, DEFERRED_COLOR_FORMAT, MeshletCull, MeshletCullPipelines,
    MeshletDeferredShader, MeshletVisRasterizer, VISIBILITY_BUFFER_FORMAT, build_default_meshlets,
    meshlet_bind_group, meshlet_bind_group_layout,
};

const RT_WIDTH: u32 = 64;
const RT_HEIGHT: u32 = 64;
const ROW_BYTES: u32 = 64 * 4;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn render_with_material(material_id: u32) -> Vec<u8> {
    let Some((device, queue)) = try_acquire_device() else {
        return Vec::new();
    };

    let mesh = build_cube_mesh();
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let gpu_mesh = meshlet_mesh.upload(&device);

    let cull = MeshletCull::new(
        &device,
        gpu_mesh.meshlet_count.max(1) * 2,
        DEFAULT_MAX_TRIANGLES as u32,
    );
    let cull_pipelines = MeshletCullPipelines::new(&device);
    let vbuf_raster = MeshletVisRasterizer::new(
        &device,
        Some(DEPTH_FORMAT),
        cull_pipelines.meshlet_bind_group_layout(),
        None,
    );
    let deferred = MeshletDeferredShader::new(&device, cull_pipelines.meshlet_bind_group_layout());

    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = meshlet_bind_group(&device, &meshlet_bgl, &gpu_mesh);

    // slot 0 is the MaterialPipeline fallback (white). Pure-red and
    // pure-blue land in slots 1 and 2 — see the call sites below.
    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    materials.register(
        &queue,
        Guid::new_v4(),
        &Material::new([1.0, 0.0, 0.0, 1.0], 0.0, 0.5, 0.0),
    );
    materials.register(
        &queue,
        Guid::new_v4(),
        &Material::new([0.0, 0.0, 1.0, 1.0], 0.0, 0.5, 0.0),
    );
    let material_bg = materials.pool().bind_group(&device);

    let vbuf_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vbuf_mat_test"),
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
        label: Some("color_mat_test"),
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
        label: Some("depth_mat_test"),
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
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view_proj = proj * view;
    let model = Mat4::IDENTITY;
    let cull_params = CullParams::new(view_proj, cam, gpu_mesh.meshlet_count);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("mat_test_encoder"),
    });
    cull.dispatch(
        &cull_pipelines,
        &device,
        &queue,
        &mut encoder,
        &gpu_mesh,
        &cull_params,
    );
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
        material_id,
    );

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mat_test_staging"),
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
    slice.get_mapped_range().to_vec()
}

#[test]
fn distinct_material_ids_produce_distinct_pixel_colors() {
    let red_pixels = render_with_material(1);
    let blue_pixels = render_with_material(2);

    if red_pixels.is_empty() || blue_pixels.is_empty() {
        eprintln!("no GPU adapter available; skipping");
        return;
    }

    // Find one non-clear pixel in the red render whose RGB channel
    // ordering signals red dominance (R > B, R > 0).
    let mut found_red = false;
    for chunk in red_pixels.chunks_exact(4) {
        let r = chunk[0] as u32;
        let g = chunk[1] as u32;
        let b = chunk[2] as u32;
        if r > 0 && r > b + 16 {
            // Material is pure red, so green/blue should be heavily
            // attenuated. Tolerance accounts for normal-debug shading
            // (red = normal_debug.r * 1.0; green/blue zero).
            assert!(
                g < r,
                "red material should not raise green above red (saw r={r}, g={g}, b={b})"
            );
            found_red = true;
            break;
        }
    }
    assert!(
        found_red,
        "red material id should produce some red-dominant pixels"
    );

    // And the blue render should produce blue-dominant pixels.
    let mut found_blue = false;
    for chunk in blue_pixels.chunks_exact(4) {
        let r = chunk[0] as u32;
        let g = chunk[1] as u32;
        let b = chunk[2] as u32;
        if b > 0 && b > r + 16 {
            assert!(
                g < b,
                "blue material should not raise green above blue (saw r={r}, g={g}, b={b})"
            );
            found_blue = true;
            break;
        }
    }
    assert!(
        found_blue,
        "blue material id should produce some blue-dominant pixels"
    );

    // Sanity: red pixels are not the same as blue pixels.
    assert_ne!(red_pixels, blue_pixels);
}
