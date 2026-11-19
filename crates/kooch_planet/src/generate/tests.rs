use glam::IVec3;

use super::{BARE_SPHERE as SPHERE, GeneratePass, Job};
use crate::TILE_SAMPLES;
use crate::test_device::{read_tile, try_acquire};
use crate::{BlockAtlas, BlockKey, Shape};

const EARTH: f64 = 6_371_000.0;

fn earth() -> Shape {
    Shape {
        radius: EARTH,
        relief: 9_000.0,
        depth: 2_000.0,
        voxel: 0.5,
    }
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
