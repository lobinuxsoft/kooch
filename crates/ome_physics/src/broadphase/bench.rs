//! `#[ignore]`-gated diagnostic benches for [`super::BroadphasePairs`].
//! Run with `cargo test -p ome_physics --lib bench_broadphase --
//! --ignored --nocapture` to see throughput + false-positive ratio
//! on stdout.

use glam::Vec3;
use ome_bvh::{Aabb, Bvh, LeafAabb};

use super::tests::collider_leaf;
use super::BroadphasePairs;

/// 1000 random colliders in a 10×10×10 box, half-extent 0.3 →
/// moderate overlap density. Reports pairs/ms throughput averaged
/// over 100 runs to amortise allocator state across measurements.
#[test]
#[ignore = "diagnostic bench — run with --ignored --nocapture"]
fn bench_broadphase_1k_colliders_throughput() {
    let mut rng_state = 0xBEEFC0DEu32;
    let mut rand = || {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) as f32 / 32768.0
    };
    let leaf_aabbs: Vec<LeafAabb> = (0..1000u32)
        .map(|i| {
            let p = Vec3::new(rand(), rand(), rand()) * 10.0;
            collider_leaf(p, 0.3, i)
        })
        .collect();
    let items: Vec<(u32, Aabb)> = leaf_aabbs
        .iter()
        .enumerate()
        .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
        .collect();
    let bvh = Bvh::build(items);

    let runs = 100;
    let t = std::time::Instant::now();
    let mut last_len = 0;
    for _ in 0..runs {
        let pairs = BroadphasePairs::from_cpu_mirror(&bvh, &leaf_aabbs);
        last_len = pairs.len();
    }
    let elapsed = t.elapsed();
    let per_run_ms = elapsed.as_secs_f64() * 1000.0 / runs as f64;
    let pairs_per_ms = last_len as f64 / per_run_ms;
    eprintln!(
        "[broadphase bench] N=1000 pairs={last_len} per_run={per_run_ms:.3}ms \
         throughput={pairs_per_ms:.0} pairs/ms (averaged over {runs} runs)",
    );
}

/// Builds two parallel scenes from the same centres — one with **tight**
/// per-role half-extents and one with **envelope** half-extents (tight +
/// `k_role_max` inflation). Reports the ratio
/// `pairs(envelope) / pairs(tight)` — the inflation cost the broadphase
/// pays when entities also participate in raymarch smooth-blends.
/// >1.5× justifies the per-role-AABB follow-up filed by S7 of #115 PR-5;
/// <1.5× = low-prio.
#[test]
#[ignore = "diagnostic bench — run with --ignored --nocapture"]
fn bench_broadphase_false_positives_envelope_vs_tight() {
    // Synthetic k_role_max — chosen to be roughly 30% of the tight
    // half-extent so the envelope test has meaningful inflation.
    // Real engine values come from the per-scene CSG smoothing
    // constants; the bench here just demonstrates the methodology.
    let tight_half = 0.3;
    let k_role_max = 0.1;
    let envelope_half = tight_half + k_role_max;

    let mut rng_state = 0xC011DEFEu32;
    let mut rand = || {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) as f32 / 32768.0
    };

    let n = 1000u32;
    let centres: Vec<Vec3> = (0..n)
        .map(|_| Vec3::new(rand(), rand(), rand()) * 10.0)
        .collect();

    let make_scene = |half: f32| -> (Bvh<u32>, Vec<LeafAabb>) {
        let leaves: Vec<LeafAabb> = centres
            .iter()
            .enumerate()
            .map(|(i, c)| collider_leaf(*c, half, i as u32))
            .collect();
        let items: Vec<(u32, Aabb)> = leaves
            .iter()
            .enumerate()
            .map(|(i, la)| (i as u32, Aabb::new(la.aabb_min.into(), la.aabb_max.into())))
            .collect();
        (Bvh::build(items), leaves)
    };

    let (bvh_tight, leaves_tight) = make_scene(tight_half);
    let (bvh_envelope, leaves_envelope) = make_scene(envelope_half);

    let pairs_tight = BroadphasePairs::from_cpu_mirror(&bvh_tight, &leaves_tight).len();
    let pairs_envelope =
        BroadphasePairs::from_cpu_mirror(&bvh_envelope, &leaves_envelope).len();

    let ratio = if pairs_tight == 0 {
        f64::INFINITY
    } else {
        pairs_envelope as f64 / pairs_tight as f64
    };
    let verdict = if ratio > 1.5 {
        "FOLLOW-UP: tighter per-role AABBs justified (>1.5× envelope/tight)"
    } else {
        "OK: envelope inflation cost is below the 1.5× action threshold"
    };
    eprintln!(
        "[broadphase false-positives bench] N={n} tight_half={tight_half} \
         k_role_max={k_role_max} pairs(tight)={pairs_tight} pairs(envelope)={pairs_envelope} \
         ratio={ratio:.3} → {verdict}",
    );
}
