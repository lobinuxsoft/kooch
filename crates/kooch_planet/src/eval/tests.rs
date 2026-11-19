use glam::DVec3;

use super::{Generator, may_cross};
use crate::generate::{GeneratePass, Job};
use crate::select::{bounds, voxel_size};
use crate::test_device::{read_tile, try_acquire};
use crate::{BARE_SPHERE, BlockAtlas, BlockKey, Shape};

/// The shader graph's hash on `sin`, whose GPU precision WGSL leaves open (#1930), summed in a loop.
const NOISY: &str = r#"
fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x0 = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x1 = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x2 = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x3 = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x0, x1, u.y), mix(x2, x3, u.y), u.z);
}
fn planet(input: PlanetInput) -> f32 {
    var h = 0.0;
    var amplitude = 1.0;
    var p = input.direction * 40.0;
    for (var octave = 0; octave < 4; octave++) {
        h += amplitude * noise(p);
        amplitude *= 0.5;
        p *= 2.0;
    }
    return input.altitude - 6.0 * h;
}
"#;

fn small() -> Shape {
    Shape {
        radius: 200.0,
        relief: 12.0,
        depth: 4.0,
        voxel: 0.5,
    }
}

/// The tile box `crosses` bounds: the block and one sample past it on every side.
fn tile_range(generator: &Generator, shape: &Shape, key: BlockKey) -> Option<(f64, f64)> {
    let (min, max) = bounds(shape, key);
    let pad = DVec3::splat(voxel_size(shape, key.level));
    generator.range(shape, min - pad, max + pad, key.planet)
}

fn key_at(shape: &Shape, point: DVec3) -> BlockKey {
    let side = voxel_size(shape, 0) * 16.0;
    BlockKey {
        planet: 0,
        level: 0,
        cell: (point / side).floor().as_ivec3(),
    }
}

/// The bare sphere bounds a block in the sky above zero and one on the ground across it.
#[test]
fn a_sphere_is_bounded() {
    let shape = small();
    let generator = Generator::parse(BARE_SPHERE).unwrap();
    let sky = key_at(&shape, DVec3::new(0.0, 230.0, 0.0));
    let ground = key_at(&shape, DVec3::new(0.0, 200.0, 0.0));
    let up = tile_range(&generator, &shape, sky).unwrap();
    assert!(up.0 > 0.0, "{up:?}");
    assert!(!may_cross(Some(up), shape.voxel));
    assert!(may_cross(
        tile_range(&generator, &shape, ground),
        shape.voxel
    ));
}

/// Every sample the GPU generates lies in the range the CPU bounds its tile by; a block the
/// selection skips on that range would otherwise have held surface.
#[test]
fn gpu_samples_stay_in_range() {
    let Some((device, queue)) = try_acquire() else {
        return;
    };
    let shape = small();
    let generator = Generator::parse(NOISY).unwrap();
    let atlas = BlockAtlas::new(&device, 64);
    let pass = GeneratePass::new(&device, NOISY, &atlas, 16).unwrap();
    let directions = [
        DVec3::new(1.0, 0.3, -0.2),
        DVec3::new(-0.4, 1.0, 0.7),
        DVec3::new(0.2, -0.6, -1.0),
        DVec3::new(-1.0, -1.0, 1.0),
    ];
    let mut keys = Vec::new();
    for direction in directions {
        for height in [-2.0, 3.0, 8.0, 14.0] {
            keys.push(key_at(
                &shape,
                direction.normalize() * (shape.radius + height),
            ));
        }
    }
    let jobs: Vec<Job> = (0u32..)
        .zip(&keys)
        .map(|(slot, &k)| Job::new(k, slot, &shape))
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.record(&device, &queue, &mut encoder, &atlas, &jobs, None);
    queue.submit([encoder.finish()]);

    let mut bounded = 0;
    for (slot, key) in (0u32..).zip(&keys) {
        let Some((lo, hi)) = tile_range(&generator, &shape, *key) else {
            continue;
        };
        bounded += 1;
        let voxel = shape.voxel;
        for got in read_tile(&device, &queue, &atlas, slot) {
            // f16 holds 11 bits; a clamp to ±64 voxels only moves a sample inward.
            let slack = f64::from(got.abs()) / 1024.0;
            let got = f64::from(got);
            assert!(got <= hi / voxel + slack, "{key:?} {got} > {hi}");
            assert!(got >= lo / voxel - slack, "{key:?} {got} < {lo}");
        }
    }
    assert_eq!(bounded, keys.len(), "the noise was not bounded");
}
