use crate::aabb::Aabb;

use glam::Vec3;

use super::helpers::{aabb_at, random_items};
use super::refit_helpers::{perturb_translate, run_refit_pair};

#[test]
fn refit_gpu_matches_cpu_n_1() {
    let items = vec![(0u32, aabb_at(Vec3::ZERO, 1.0))];
    let moved = vec![(0u32, aabb_at(Vec3::splat(0.3), 1.0))];
    run_refit_pair(items, moved, "refit_n=1");
}

#[test]
fn refit_gpu_matches_cpu_n_2() {
    let items = vec![
        (0u32, aabb_at(Vec3::ZERO, 0.5)),
        (1u32, aabb_at(Vec3::splat(10.0), 0.5)),
    ];
    let moved = vec![
        (0u32, aabb_at(Vec3::splat(0.05), 0.55)),
        (1u32, aabb_at(Vec3::splat(10.05), 0.45)),
    ];
    run_refit_pair(items, moved, "refit_n=2");
}

#[test]
fn refit_gpu_matches_cpu_n_8() {
    let items: Vec<(u32, Aabb)> = (0..8u32)
        .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
        .collect();
    let moved = perturb_translate(&items, Vec3::new(0.05, 0.02, -0.03));
    run_refit_pair(items, moved, "refit_n=8");
}

#[test]
fn refit_gpu_matches_cpu_n_1024() {
    let items: Vec<(u32, Aabb)> = (0..1024u32)
        .map(|i| {
            let x = (i % 32) as f32;
            let y = (i / 32) as f32;
            (i, aabb_at(Vec3::new(x, y, 0.0), 0.4))
        })
        .collect();
    let moved = perturb_translate(&items, Vec3::new(0.05, -0.05, 0.02));
    run_refit_pair(items, moved, "refit_n=1024");
}

#[test]
fn refit_gpu_matches_cpu_random_n_100() {
    let items = random_items(100, 0xc0ffee01, 10.0);
    let moved = perturb_translate(&items, Vec3::new(0.03, 0.04, -0.02));
    run_refit_pair(items, moved, "refit_n=100");
}
