use std::collections::HashMap;

use glam::{DVec3, IVec3};

use crate::planet::{
    BARE_SPHERE, BlockKey, BlockPool, Limits, MeshedBlock, Planets, Residency, Shape, Terrain,
};
use crate::voxel::test_device::try_acquire;

const RADIUS: f64 = 6.0;

/// The eight level-0 blocks around the centre hold the whole sphere.
fn sphere_blocks() -> (Planets, Vec<BlockKey>) {
    let mut planets = Planets::default();
    let shape = Shape {
        radius: RADIUS,
        relief: 1.0,
        depth: 1.0,
        voxel: 0.5,
    };
    planets.push(
        shape,
        DVec3::ZERO,
        Limits {
            floor: 8,
            ceiling: 8,
        },
    );
    let mut keys = Vec::new();
    for i in 0..8 {
        let cell = IVec3::new(i & 1, (i >> 1) & 1, i >> 2) - IVec3::ONE;
        keys.push(BlockKey {
            planet: 0,
            level: 0,
            cell,
        });
    }
    (planets, keys)
}

fn mesh_all(device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<MeshedBlock> {
    let (planets, keys) = sphere_blocks();
    let mut pool = BlockPool::new(64);
    let mut residency = Residency::default();
    pool.apply(&keys, &mut residency);
    let mut terrain = Terrain::new(device, queue, 64, BARE_SPHERE).unwrap();
    let mut out = Vec::new();
    let mut all = Vec::new();
    for _ in 0..8 {
        terrain.update(device, queue, &pool, &planets, &residency, &mut out);
        residency = Residency::default();
        all.append(&mut out);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        if all.len() == keys.len() {
            break;
        }
    }
    all
}

/// 🔴 The blocks of one level close into one surface facing out: every edge is used once each way.
/// A crack between blocks leaves an edge used once; a flipped triangle uses one twice the same way.
#[test]
fn a_meshed_sphere_is_watertight() {
    let Some((device, queue)) = try_acquire() else {
        return;
    };
    let blocks = mesh_all(&device, &queue);
    assert_eq!(blocks.len(), 8, "every block comes back");
    let quantise = |p: DVec3| (p * 1000.0).round().as_ivec3();
    let mut edges: HashMap<(IVec3, IVec3), i32> = HashMap::new();
    let mut worst = 0.0f64;
    let mut triangles = 0;
    for block in &blocks {
        let at = |i: u32| block.origin + glam::Vec3::from(block.positions[i as usize]).as_dvec3();
        for p in &block.positions {
            let world = block.origin + glam::Vec3::from(*p).as_dvec3();
            worst = worst.max((world.length() - RADIUS).abs());
        }
        for tri in block.indices.as_chunks::<3>().0 {
            let [a, b, c] = [at(tri[0]), at(tri[1]), at(tri[2])];
            let facing = (b - a).cross(c - a).dot((a + b + c) / 3.0);
            assert!(facing > 0.0, "a triangle faces into the planet");
            triangles += 1;
            for (from, to) in [(a, b), (b, c), (c, a)] {
                *edges.entry((quantise(from), quantise(to))).or_default() += 1;
            }
        }
    }
    assert!(triangles > 100, "{triangles} triangles");
    assert!(worst < 0.1, "a vertex sits {worst} m off the sphere");
    for (&(from, to), &count) in &edges {
        assert_eq!(count, 1, "an edge is used {count} times the same way");
        assert_eq!(
            edges.get(&(to, from)),
            Some(&1),
            "an edge has no twin: a crack"
        );
    }
}
