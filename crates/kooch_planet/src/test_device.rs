//! A device for this crate's GPU tests, and a way to read back what it generated.

use crate::{BlockAtlas, TILE_SAMPLES};

/// A device with the `r16float` storage atlas, or `None` so the test skips without a GPU.
pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = kooch_core::gpu::test_adapter()?;
    let required = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(required) {
        eprintln!("skipping planet GPU test: adapter missing {required:?}");
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kooch_planet::test_device"),
        required_features: required,
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// IEEE half to f32, enough for finite values.
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    match exp {
        0 => sign * mantissa * 2f32.powi(-24),
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// One slot's tile, x fastest.
pub fn read_tile(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &BlockAtlas,
    slot: u32,
) -> Vec<f32> {
    let dim = TILE_SAMPLES;
    let row = 256u32;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * dim * dim),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let corner = atlas.corner(slot);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: atlas.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: corner.x,
                y: corner.y,
                z: corner.z,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(dim),
            },
        },
        wgpu::Extent3d {
            width: dim,
            height: dim,
            depth_or_array_layers: dim,
        },
    );
    queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    let bytes = staging.slice(..).get_mapped_range().unwrap().to_vec();
    let mut out = Vec::new();
    for z in 0..dim {
        for y in 0..dim {
            for x in 0..dim {
                let at = ((z * dim + y) * row + x * 2) as usize;
                out.push(half(u16::from_le_bytes([bytes[at], bytes[at + 1]])));
            }
        }
    }
    out
}
