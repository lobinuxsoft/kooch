use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::gpu::builder::{test_device, BvhGpuBuilder};

use super::super::full::build_gpu;

use glam::Vec3;

pub(super) fn aabb_at(centre: Vec3, half: f32) -> Aabb {
    Aabb::from_centre(centre, Vec3::splat(half))
}

/// Cheap deterministic LCG — the same constants used elsewhere in
/// the ome_bvh tests, so reproductions match.
fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1103515245).wrapping_add(12345);
    (*state >> 16) as f32 / 32768.0
}

pub(super) fn random_items(n: u32, seed: u32, world_size: f32) -> Vec<(u32, Aabb)> {
    let mut state = seed;
    (0..n)
        .map(|i| {
            let centre = Vec3::new(lcg(&mut state), lcg(&mut state), lcg(&mut state))
                * world_size;
            (i, Aabb::from_centre(centre, Vec3::splat(0.2)))
        })
        .collect()
}

fn assert_gpu_matches_cpu(gpu: &Bvh<u32>, cpu: &Bvh<u32>, label: &str) {
    assert_eq!(
        gpu.nodes.len(),
        cpu.nodes.len(),
        "[{label}] node count: gpu={} cpu={}",
        gpu.nodes.len(),
        cpu.nodes.len()
    );
    for (i, (g, c)) in gpu.nodes.iter().zip(cpu.nodes.iter()).enumerate() {
        assert_eq!(
            g, c,
            "[{label}] node[{i}] diverges:\n  gpu: {g:?}\n  cpu: {c:?}"
        );
    }
    assert_eq!(
        gpu.leaves, cpu.leaves,
        "[{label}] leaves payload mismatch"
    );
}

pub(super) fn run_pair(items: Vec<(u32, Aabb)>, label: &str) {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::build: no GPU adapter — skipping {label}");
        return;
    };
    let mut builder = BvhGpuBuilder::new(&device, &queue, None);
    let cpu = Bvh::build(items.clone());
    let build = build_gpu::<u32>(&mut builder, &device, &queue, items);
    let result = build.block_on(&device).expect("GPU build failed");
    assert_gpu_matches_cpu(&result.bvh, &cpu, label);
}
