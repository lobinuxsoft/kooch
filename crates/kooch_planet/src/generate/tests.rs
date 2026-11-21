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

/// Metre-scale noise read from `position`, where f32 is coarsest on a big planet (#1933).
const FINE: &str = concat!(
    include_str!("../../../kooch_editor_core/src/shader_graph/codegen/helpers/hash3.wgsl"),
    r#"
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x0 = mix(graph_hash3(i), graph_hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x1 = mix(graph_hash3(i + vec3<f32>(0.0, 1.0, 0.0)), graph_hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x2 = mix(graph_hash3(i + vec3<f32>(0.0, 0.0, 1.0)), graph_hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x3 = mix(graph_hash3(i + vec3<f32>(0.0, 1.0, 1.0)), graph_hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x0, x1, u.y), mix(x2, x3, u.y), u.z);
}
fn planet(input: PlanetInput) -> f32 {
    return input.altitude - 3.0 * noise(input.position * 0.25);
}
"#
);

/// Two blocks of one level evaluate the samples on their shared plane; they must agree on every
/// sign, or each contours the plane its own way and the seam cracks.
#[test]
fn neighbours_share_their_signs() {
    let Some((device, queue)) = try_acquire() else {
        return;
    };
    // A voxel that is not a power of two leaves `origin + local` inexact in f32: 30 and 411 of
    // 5832 shared samples flipped before the grid index.
    for (radius, voxel) in [(EARTH, 0.5), (EARTH, 0.3), (7.0e7, 0.3)] {
        let shape = Shape {
            voxel,
            radius,
            ..earth()
        };
        let (flipped, near) = shared_flips(&device, &queue, &shape);
        assert!(
            near > 0,
            "r {radius} voxel {voxel}: no shared sample near the surface"
        );
        assert_eq!(
            flipped, 0,
            "r {radius} voxel {voxel}: shared samples disagree on their sign"
        );
    }
}

/// Shared samples of same-level neighbours whose signs differ, and how many lie within a voxel
/// of the surface.
fn shared_flips(device: &wgpu::Device, queue: &wgpu::Queue, shape: &Shape) -> (usize, usize) {
    let shape = *shape;
    let directions = [
        glam::DVec3::new(1.0, 0.7, 0.3),
        glam::DVec3::new(-0.2, 1.0, -0.9),
        glam::DVec3::new(0.6, -0.4, 1.0),
    ];
    let mut pairs = Vec::new();
    for direction in directions {
        let cell = (direction.normalize() * shape.radius / (shape.voxel * 16.0))
            .floor()
            .as_ivec3();
        let key = BlockKey {
            planet: 0,
            level: 0,
            cell,
        };
        for axis in 0..3 {
            let mut next = key;
            next.cell[axis] += 1;
            pairs.push((key, next, axis));
        }
    }
    let atlas = BlockAtlas::new(device, 64);
    let pass = GeneratePass::new(device, FINE, &atlas, 32).unwrap();
    let jobs: Vec<Job> = (0u32..)
        .zip(pairs.iter().flat_map(|(a, b, _)| [*a, *b]))
        .map(|(slot, key)| Job::new(key, slot, &shape))
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.record(device, queue, &mut encoder, &atlas, &jobs, None);
    queue.submit([encoder.finish()]);

    let dim = TILE_SAMPLES as usize;
    let at = |p: [usize; 3]| (p[2] * dim + p[1]) * dim + p[0];
    let (mut flipped, mut near) = (0, 0);
    for (n, (_, _, axis)) in (0u32..).zip(&pairs) {
        let low = read_tile(device, queue, &atlas, 2 * n);
        let high = read_tile(device, queue, &atlas, 2 * n + 1);
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        // The low tile's last two samples along `axis` are the high tile's first two.
        for j in 0..2 {
            for a in 0..dim {
                for b in 0..dim {
                    let (mut l, mut h) = ([0; 3], [0; 3]);
                    (l[*axis], h[*axis]) = (dim - 2 + j, j);
                    (l[u], h[u], l[w], h[w]) = (a, a, b, b);
                    let (x, y) = (low[at(l)], high[at(h)]);
                    near += usize::from(x.abs() < 1.0);
                    if (x > 0.0) != (y > 0.0) {
                        flipped += 1;
                    }
                }
            }
        }
    }
    (flipped, near)
}
