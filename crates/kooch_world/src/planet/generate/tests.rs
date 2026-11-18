use glam::IVec3;

use super::{BARE_SPHERE as SPHERE, GeneratePass, Job};
use crate::planet::TILE_SAMPLES;
use crate::planet::{BlockAtlas, BlockKey, Shape};
use crate::voxel::test_device::try_acquire;

const EARTH: f64 = 6_371_000.0;

fn earth() -> Shape {
    Shape {
        radius: EARTH,
        relief: 9_000.0,
        depth: 2_000.0,
        voxel: 0.5,
    }
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
fn read_tile(
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

/// A generator that does not compile is refused before wgpu sees it.
#[test]
fn a_broken_generator_is_refused() {
    assert!(super::validate_generator(SPHERE).is_ok());
    assert!(
        super::validate_generator("fn planet(input: PlanetInput) -> f32 { return nope; }").is_err()
    );
}

/// A block on the ground of an Earth-sized planet, off the axes, where f32 `length(p) − R` is off
/// by about half a voxel.
#[test]
fn earth_altitude_stays_exact() {
    let Some((device, queue)) = try_acquire() else {
        return;
    };
    let shape = earth();
    let key = BlockKey {
        planet: 0,
        level: 0,
        cell: IVec3::splat((EARTH / 3f64.sqrt() / 8.0) as i32),
    };
    let atlas = BlockAtlas::new(&device, 1024);
    let pass = GeneratePass::new(&device, SPHERE, &atlas, 4).unwrap();
    let slot = 7;
    let job = Job::new(key, slot, &shape);
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.record(&device, &queue, &mut encoder, &atlas, &[job], None);
    queue.submit([encoder.finish()]);

    let tile = read_tile(&device, &queue, &atlas, slot);
    let origin = key.cell.as_dvec3() * 8.0 - glam::DVec3::splat(0.5);
    let dim = TILE_SAMPLES as usize;
    let mut worst = 0.0f64;
    let mut near = 0;
    for (i, &got) in tile.iter().enumerate() {
        let v = glam::DVec3::new(
            (i % dim) as f64,
            ((i / dim) % dim) as f64,
            (i / dim / dim) as f64,
        );
        let truth = ((origin + v * 0.5).length() - EARTH) / 0.5;
        if truth.abs() < 32.0 {
            worst = worst.max((f64::from(got) - truth).abs());
            near += 1;
        }
    }
    assert!(near > 0, "the block misses the surface");
    assert!(worst < 0.05, "worst error {worst} voxels");
}
