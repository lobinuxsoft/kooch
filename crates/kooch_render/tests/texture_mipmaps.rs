//! Mip chains, and the one thing they are easy to get wrong.

mod common;

use kooch_render::texture::{GpuTexture, Image, ImageFormat, Mipmapper, level_count};

/// Serialised for the reason the other GPU binaries are: `common` hands
/// every case the same device, and concurrent submission against it
/// segfaults radv rather than failing a case.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

/// Reads one mip level back as RGBA8.
fn read_level(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
) -> Vec<u8> {
    let size = texture
        .size()
        .mip_level_size(level, wgpu::TextureDimension::D2);
    let (w, h) = (size.width, size.height);
    let padded = (w * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mip_readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("mip_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
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
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    staging.unmap();
    out
}

/// A 2x2 black-and-white checker, which is the smallest thing whose
/// average is interesting.
fn checker(format: ImageFormat) -> Image {
    let px = [
        0u8, 0, 0, 255, 255, 255, 255, 255, //
        255, 255, 255, 255, 0, 0, 0, 255,
    ];
    Image::from_rgba8(px.to_vec(), 2, 2, format)
}

/// 🔴 The chain averages in LINEAR light, not in gamma-encoded bytes.
#[test]
fn the_chain_averages_in_linear_light() {
    let _gpu = gpu_lock();
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut mipmapper = Mipmapper::new(&device);
    let texture = GpuTexture::upload_with(
        &device,
        &queue,
        &checker(ImageFormat::Rgba8UnormSrgb),
        &mut mipmapper,
    );
    assert_eq!(texture.texture.mip_level_count(), 2);

    let last = read_level(&device, &queue, &texture.texture, 1);
    eprintln!("2x2 sRGB checker, mip 1 = {:?}", &last[..4]);
    for c in &last[..3] {
        assert!(
            (*c as i32 - 188).abs() <= 2,
            "mip 1 came out at {c}, not 188. 128 means the four texels were averaged as \
             bytes instead of as light, which darkens every distant surface in the scene",
        );
    }
}

/// And a linear texture averages its bytes, because there the bytes ARE the quantity.
#[test]
fn a_linear_texture_averages_its_bytes() {
    let _gpu = gpu_lock();
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut mipmapper = Mipmapper::new(&device);
    let texture = GpuTexture::upload_with(
        &device,
        &queue,
        &checker(ImageFormat::Rgba8Unorm),
        &mut mipmapper,
    );

    let last = read_level(&device, &queue, &texture.texture, 1);
    eprintln!("2x2 linear checker, mip 1 = {:?}", &last[..4]);
    for c in &last[..3] {
        assert!(
            (*c as i32 - 128).abs() <= 2,
            "mip 1 came out at {c}, not 128 — a linear texture was decoded as if it \
             carried a gamma curve",
        );
    }
}

/// The whole chain is written, not just the first level.
#[test]
fn every_level_is_written() {
    let _gpu = gpu_lock();
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let side = 64;
    let px = vec![255u8; side * side * 4];
    let image = Image::from_rgba8(px, side as u32, side as u32, ImageFormat::Rgba8Unorm);
    let mut mipmapper = Mipmapper::new(&device);
    let texture = GpuTexture::upload_with(&device, &queue, &image, &mut mipmapper);

    let levels = level_count(side as u32, side as u32);
    assert_eq!(
        texture.texture.mip_level_count(),
        levels,
        "wrong chain length"
    );
    for level in 0..levels {
        let data = read_level(&device, &queue, &texture.texture, level);
        assert!(
            data.iter().all(|c| *c == 255),
            "level {level} of a solid white texture is not solid white — it was never \
             written, or it was written from the wrong source level",
        );
    }
}

/// And an import that says no gets exactly one level.
#[test]
fn an_import_can_refuse_the_chain() {
    let _gpu = gpu_lock();
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let mut mipmapper = Mipmapper::new(&device);
    let image = checker(ImageFormat::Rgba8UnormSrgb).without_mipmaps();
    let texture = GpuTexture::upload_with(&device, &queue, &image, &mut mipmapper);
    assert_eq!(texture.texture.mip_level_count(), 1);
}

/// 🔴 Level zero survives, and the levels above it lose detail in order.
#[test]
fn the_chain_loses_detail_in_order() {
    let _gpu = gpu_lock();
    let Some((device, queue)) = common::try_acquire_device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // 8x8, one-pixel checker: four levels, and the last is 1x1.
    let side = 8usize;
    let mut px = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let v = if (x + y) % 2 == 0 { 0u8 } else { 255 };
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let image = Image::from_rgba8(px, side as u32, side as u32, ImageFormat::Rgba8Unorm);
    let mut mipmapper = Mipmapper::new(&device);
    let texture = GpuTexture::upload_with(&device, &queue, &image, &mut mipmapper);
    assert_eq!(texture.texture.mip_level_count(), 4);

    let spread = |data: &[u8]| -> u32 {
        let reds: Vec<u8> = data.chunks_exact(4).map(|p| p[0]).collect();
        u32::from(*reds.iter().max().unwrap()) - u32::from(*reds.iter().min().unwrap())
    };

    let level_0 = read_level(&device, &queue, &texture.texture, 0);
    eprintln!("level 0 spread {}", spread(&level_0));
    assert_eq!(
        spread(&level_0),
        255,
        "level zero is no longer the image that was uploaded — something averaged it, \
         and a texture like that looks identical at every distance",
    );

    let mut previous = spread(&level_0);
    for level in 1..4 {
        let data = read_level(&device, &queue, &texture.texture, level);
        let s = spread(&data);
        eprintln!("level {level} spread {s}");
        assert!(
            s <= previous,
            "level {level} has MORE contrast than the level below it, so it was not \
             built from it",
        );
        previous = s;
    }
    assert_eq!(
        previous, 0,
        "the last level is 1x1 and cannot have a spread"
    );
}
