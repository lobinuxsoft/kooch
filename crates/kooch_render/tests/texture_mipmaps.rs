//! Mip chains, and the one thing they are easy to get wrong.
//!
//! Run with:
//!   cargo test -p kooch_render --test texture_mipmaps

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
///
/// Half black and half white is 0.5 of the light, and 0.5 of the light
/// is **188** written back as sRGB — not 128. Averaging the bytes gives
/// 128, which is 0.216 of the light: every distant surface comes out
/// visibly darker than the one next to it, and the seam moves with the
/// camera. It is the classic mip bug and it looks like a lighting
/// problem, which is where the search would go.
///
/// This is the assertion that pays for the pass being a render pass:
/// sampling an sRGB view decodes and writing an sRGB attachment
/// re-encodes, both in hardware.
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

/// And a linear texture averages its bytes, because there the bytes ARE
/// the quantity.
///
/// A normal map or a metal/roughness pair is data, not colour: putting
/// it through a transfer function it never had would bend the mid tones
/// of a normal and tilt every distant surface's lighting.
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
///
/// A loop that stops one short leaves the smallest levels holding
/// whatever the allocation had in it, and those are exactly the levels a
/// surface at the horizon samples.
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
///
/// The setting is what a UI atlas and a lookup table need: a chain there
/// is memory spent to make a 1:1 sample blurrier at glancing angles.
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
