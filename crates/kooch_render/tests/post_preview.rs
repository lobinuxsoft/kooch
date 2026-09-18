//! GPU integration test: the Shader Graph's post-process preview draws the effect over its test
//! image (#1201).

mod common;

use common::try_acquire_device;
use kooch_render::material::{
    MaterialParams, MaterialPool, MaterialTexturePool, Shader, TextureRef,
};
use kooch_render::post_process::{PostPreview, PreviewMaterials};

const SIDE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Renders `source` through the preview and returns the target's pixels.
fn preview(source: &str) -> Option<Vec<u8>> {
    let (device, queue) = try_acquire_device()?;
    let shader = Shader::parse(source).unwrap();
    let pool = MaterialPool::new(&device, &[MaterialParams::default()]);
    let textures = MaterialTexturePool::new(&device, &queue);
    let slots = [TextureRef::default(); 4];
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("post_preview_target"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut post = PostPreview::new(&device, &queue, FORMAT);
    let mut encoder = device.create_command_encoder(&Default::default());
    post.render(
        &device,
        &queue,
        &mut encoder,
        &view,
        (SIDE, SIDE),
        &shader.params_wgsl(),
        &shader.source,
        PreviewMaterials {
            pool: &pool,
            textures: &textures,
            slots: &slots,
        },
        0.0,
    )
    .unwrap();

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("post_preview_readback"),
        size: (SIDE * SIDE * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
        },
        wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let pixels = readback.slice(..).get_mapped_range().to_vec();
    Some(pixels)
}

/// An identity effect shows the test image: red at the top-left, where the hue sweep starts.
#[test]
fn the_test_image_shows_through() {
    let source = "// kind: post_process\nfn post_process(input: SurfaceInput) -> vec4<f32> {\n    return sample_scene(input.uv);\n}";
    let Some(pixels) = preview(source) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let top_left = &pixels[0..4];
    assert!(
        top_left[0] > 200 && top_left[1] < 40 && top_left[2] < 40,
        "{top_left:?}"
    );
}

/// The effect is what is drawn, not the image under it.
#[test]
fn the_effect_is_applied() {
    let source = "// kind: post_process\nfn post_process(input: SurfaceInput) -> vec4<f32> {\n    return vec4<f32>(1.0) - sample_scene(input.uv);\n}";
    let Some(pixels) = preview(source) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let top_left = &pixels[0..4];
    assert!(
        top_left[0] < 40 && top_left[1] > 200 && top_left[2] > 200,
        "{top_left:?}"
    );
}
