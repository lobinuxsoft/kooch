//! `#[ignore]`-gated diagnostic benches for [`FrustumCull`]. Live in
//! their own module so the correctness suite in `tests.rs` stays under
//! the no-monolithic threshold.
//!
//! Run with `cargo test -p ome_render --lib bench_frustum -- --ignored
//! --nocapture` to see the timings + visibility ratios on stdout.

use glam::Vec3;
use ome_bvh::SharedBvhState;

use super::cull::FrustumCull;
use super::tests::{
    axis_aligned_box_frustum, dispatch_and_readback, drive_build_to_completion,
    try_acquire_device, visible_mesh_scene,
};

/// 10k-cube planar slice — useful when tuning the right plane
/// position or sanity-checking shader regressions. Reports
/// visible/total ratio and dispatch + readback time.
#[test]
#[ignore = "diagnostic bench — run with --ignored --nocapture"]
fn bench_frustum_cull_10k_cubes() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("frustum_cull bench: no GPU adapter — skipping");
        return;
    };
    let mut centres = Vec::with_capacity(10_000);
    for j in 0..100 {
        for i in 0..100 {
            centres.push(Vec3::new(i as f32, j as f32, 0.0));
        }
    }
    let (items, leaves) = visible_mesh_scene(&centres, 0.4);
    let n = items.len() as u32;
    let planes = axis_aligned_box_frustum(
        Vec3::new(-1000.0, -1000.0, -1000.0),
        Vec3::new(49.5, 1000.0, 1000.0),
    );

    let mut shared = SharedBvhState::new(&device, &queue, None);
    let _ = shared.kick(&device, &queue, items, leaves, /* hash */ 1);
    drive_build_to_completion(&mut shared, &device, &queue);

    let mut cull = FrustumCull::new(&device, None);

    // Time a single dispatch + readback round-trip. The hot loop in
    // production binds the buffer directly via draw_indexed_indirect
    // and avoids the readback entirely; the readback time is a test-
    // only artifact, not a meaningful production metric.
    let t = std::time::Instant::now();
    let args = dispatch_and_readback(&device, &queue, &mut cull, &shared, &planes, n);
    let elapsed = t.elapsed();
    let visible = args.iter().filter(|a| a.instance_count == 1).count();
    eprintln!(
        "[frustum_cull bench] N={n} visible={visible}/{n} ratio={:.3} dispatch+readback={:.2?}",
        visible as f32 / n as f32,
        elapsed,
    );
}
