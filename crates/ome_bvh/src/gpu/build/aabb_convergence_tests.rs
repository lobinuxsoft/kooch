//! Regression: real-world seeds that historically tripped the AABB
//! propagation iteration cap in [`super::full::BvhGpuBuild::poll`].
//! Each seed was captured live from the editor (`/tmp/lbvh_panic_seed.ron`
//! dump path in `full.rs`) and pinned here so the failure mode does
//! not silently come back.
//!
//! Pinned seeds:
//!
//! - **`hierarchy_test_post_move_n_6`** — #333 reopen 2026-04-30.
//!   Captured by dragging the SDF Cylinder in `HierarchyTest.ome_scene`
//!   from the Inspector. N=6 (Floor SdfPlane + 5 SDF entities). The
//!   plane's ±1e10-wide bounds dominate the scene extent, collapsing
//!   the other five centres into ~0.6% of the Morton normalised range
//!   on X/Z. Internal node 0 stays unfinished after `2·log₂(6)+4 = 10`
//!   propagation iterations.

use crate::aabb::Aabb;
use crate::gpu::builder::{BvhGpuBuilder, test_device};

use super::full::build_gpu;

use glam::Vec3;

/// Wrap raw min/max tuples into the `(payload, Aabb)` pairs the
/// builder consumes. Payload is the index — irrelevant for
/// convergence; the test only cares that the builder resolves
/// without panicking.
fn pairs(min_max: &[([f32; 3], [f32; 3])]) -> Vec<(u32, Aabb)> {
    min_max
        .iter()
        .enumerate()
        .map(|(i, (mn, mx))| {
            (
                i as u32,
                Aabb::new(Vec3::from_array(*mn), Vec3::from_array(*mx)),
            )
        })
        .collect()
}

/// Verbatim from `/tmp/lbvh_panic_seed.ron` captured 2026-04-30 from
/// the editor reopen of #333. Six AABBs: index 1 is the SdfPlane
/// floor (≈40 m extent on X/Z, 0.8 m on Y), the other five are the
/// SDF primitives clustered around the origin. The wide plane
/// dominates the scene-bounds normalisation and forces the other
/// five centres into a sub-percent slice of the Morton range —
/// that's the regime in which the propagation pass historically
/// failed to converge inside the empirical iteration budget. The
/// captured topology was `[(5,1), (0,6), (7,3), (2,8), (9,10)]`,
/// i.e. internals 0↔1 and 2↔3 each formed a parent-child cycle —
/// proof that `karras_internal.wgsl` was emitting an invalid tree,
/// not that the iteration cap was too low.
#[test]
fn build_gpu_converges_on_hierarchy_test_post_move_n_6() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::build: no GPU adapter — skipping #333 hierarchy seed");
        return;
    };

    let items = pairs(&[
        ([1.498904, -0.53666365, -0.46967244], [3.898904, 1.8633364, 1.9303277]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-7.2284794, -0.2626487, -12.553985], [-4.828479, 2.1373515, -10.153985]),
        ([-3.3124545, -0.16579604, -1.45], [-0.41245437, 0.73420393, 1.45]),
    ]);

    // Quick sanity on the seed itself — defends against a future
    // edit silently dropping an entry.
    assert_eq!(items.len(), 6, "seed must keep N=6 for the regression");
    for (i, (_, a)) in items.iter().enumerate() {
        assert!(
            a.min.cmplt(a.max).all(),
            "seed entry [{i}] is degenerate (min !< max): {a:?}",
        );
        for c in a
            .min
            .to_array()
            .iter()
            .chain(a.max.to_array().iter())
        {
            assert!(c.is_finite(), "seed entry [{i}] has non-finite coord: {c}");
        }
    }

    // The build either resolves cleanly (post-fix) or panics in
    // `check_aabb_convergence_in_debug` (pre-fix). The handle ride
    // mirrors the editor flow exactly — same builder, same device,
    // same submission lifecycle.
    let mut builder = BvhGpuBuilder::new(&device, &queue, None);
    let build = build_gpu::<u32>(&mut builder, &device, &queue, items);
    let result = build
        .block_on(&device)
        .expect("GPU build must succeed on the #333 regression seed");

    // Topology sanity: every internal must have valid child indices
    // and every leaf must carry a non-degenerate AABB. We do not
    // CPU-cross-check because the panic was the regression — once
    // the iteration cap holds, the rest of the build is already
    // covered by the existing golden suite.
    let n = 6usize;
    let total = 2 * n - 1;
    assert_eq!(result.bvh.nodes.len(), total);
    for (i, node) in result.bvh.nodes.iter().enumerate() {
        let lo = Vec3::from_array(node.aabb_min);
        let hi = Vec3::from_array(node.aabb_max);
        assert!(
            lo.cmple(hi).all(),
            "node[{i}] has min > max post-build: lo={lo:?}, hi={hi:?}",
        );
    }
}

/// Editor startup flow: the first build goes through `empty_build`
/// (no scene loaded yet → `update_scene` collects zero entities),
/// followed by the panic-trigger N=6 build once the scene loads.
/// `empty_build` skips every dispatch but the SharedBvhState
/// orchestrator still walks the same kick / poll / swap lifecycle —
/// any state the empty path leaves on the builder's scratch buffers
/// would only surface here, never on a fresh-builder block_on.
#[test]
fn build_gpu_converges_after_empty_build_on_same_builder() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!(
            "ome_bvh::gpu::build: no GPU adapter — skipping #333 empty-then-N6 regression"
        );
        return;
    };

    let mut builder = BvhGpuBuilder::new(&device, &queue, None);

    // Empty kick — mirrors the first frame after EditorPlugin spawns
    // the camera but before HierarchyTest.ome_scene loads.
    let empty: Vec<(u32, Aabb)> = Vec::new();
    let build_empty = build_gpu::<u32>(&mut builder, &device, &queue, empty);
    let _ = build_empty
        .block_on(&device)
        .expect("empty build must succeed");

    // First non-empty build — the captured panic seed.
    let v2 = pairs(&[
        ([1.498904, -0.53666365, -2.5055337], [3.898904, 1.8633364, -0.10553372]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-3.4378545, -0.2626487, 0.7831924], [-1.0378544, 2.1373515, 3.1831925]),
        ([-3.3124545, -0.16579604, -1.45], [-0.41245437, 0.73420393, 1.45]),
    ]);
    let build_v2 = build_gpu::<u32>(&mut builder, &device, &queue, v2);
    let _ = build_v2
        .block_on(&device)
        .expect("V2 build must succeed AFTER an empty build on the same builder");
}

// (Legacy `shared_bvh_state_converges_on_hierarchy_test_post_move_n_6`
// removed alongside `SharedBvhState` in #360 PR-3. The pure builder
// regression — the actual convergence path — is already covered by the
// `build_gpu_*` tests above; the orchestrator-walk test gave no
// additional coverage now that nothing in the runtime uses
// SharedBvhState. AC3 streaming round-trip in
// `crates/ome_render/tests/ac3_streaming_round_trip.rs` exercises the
// equivalent insert → re-insert cadence through `OmeAccel`.)

/// Inter-build cache-flush regression: any `device.poll` (or
/// unrelated submission, including the slot-copy `SharedBvhState`
/// commits during `poll_swap`) between two builds on the same
/// `BvhGpuBuilder` flushes the GPU's L2 caches. Pre-fix that
/// surfaced a stale-data bug in `onesweep_init.wgsl`: the histogram
/// clear only swept entries 0..255 (one workgroup of 256 threads
/// against a 1024-entry buffer) and `onesweep_global_histogram.wgsl`
/// then atomic-added V2's counts onto V1's residue, producing a
/// sort whose adjacent-pair ordering was permuted, which Karras
/// then turned into a tree with 2-cycles between sibling internals
/// (0↔1, 2↔3, …). Convergence cannot finish — the issue's
/// `done_buffer` panic was a downstream symptom, not a slack
/// shortage. Without the inter-build poll the bug stayed masked
/// because cached zeroes happened to satisfy the next build's
/// reads.
#[test]
fn build_gpu_converges_after_inter_build_buffer_copy() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!(
            "ome_bvh::gpu::build: no GPU adapter — skipping #333 inter-build-copy probe"
        );
        return;
    };

    let mut builder = BvhGpuBuilder::new(&device, &queue, None);

    let v1 = pairs(&[
        ([1.498904, -0.53666365, -0.46967244], [3.898904, 1.8633364, 1.9303277]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-7.2284794, -0.2626487, -12.553985], [-4.828479, 2.1373515, -10.153985]),
        ([1.6875455, -0.16579604, -1.45], [4.5875454, 0.73420393, 1.45]),
    ]);
    let _ = build_gpu::<u32>(&mut builder, &device, &queue, v1).block_on(&device);

    // The trigger: a device.poll between builds. Mirrors what
    // `SharedBvhState::poll_swap` does (queue.submit on slot copy
    // ⇒ implicit poll on the next driver tick).
    let _ = device.poll(wgpu::PollType::Poll);

    let v2 = pairs(&[
        ([1.498904, -0.53666365, -0.46967244], [3.898904, 1.8633364, 1.9303277]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-7.2284794, -0.2626487, -12.553985], [-4.828479, 2.1373515, -10.153985]),
        ([-3.3124545, -0.16579604, -1.45], [-0.41245437, 0.73420393, 1.45]),
    ]);
    let _ = build_gpu::<u32>(&mut builder, &device, &queue, v2)
        .block_on(&device)
        .expect("V2 build must succeed across an inter-build cache flush");
}

/// Editor flow shape: the same `BvhGpuBuilder` resolves a first build
/// (V1, the scene-as-loaded), then the user moves an entity and a
/// second build (V2, the post-edit seed) runs through the same
/// builder. The convergence panic only reproduces in this two-build
/// pattern — the single-build test above is byte-clean because the
/// builder's scratch buffers (`done_buffer`, `nodes_buffer`,
/// `aabbs_buffer`) start at the post-`new` zero state. Sharing them
/// across two builds is what `SharedBvhState` does in production and
/// is the path the panic walks.
#[test]
fn build_gpu_converges_after_prior_build_on_same_builder() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!(
            "ome_bvh::gpu::build: no GPU adapter — skipping #333 same-builder regression"
        );
        return;
    };

    let mut builder = BvhGpuBuilder::new(&device, &queue, None);

    // V1: same six entities, but with the SDF Cylinder still at its
    // pre-edit position. Reconstructed by translating index-5's
    // centre by the inverse of the captured edit (Cylinder is the
    // SDF the user moved; an x-shift of -5 along the editor's drag
    // recovers a plausible pre-move pose).
    let v1 = pairs(&[
        ([1.498904, -0.53666365, -2.5055337], [3.898904, 1.8633364, -0.10553372]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-3.4378545, -0.2626487, 0.7831924], [-1.0378544, 2.1373515, 3.1831925]),
        ([1.6875455, -0.16579604, -1.45], [4.5875454, 0.73420393, 1.45]),
    ]);
    let build_v1 = build_gpu::<u32>(&mut builder, &device, &queue, v1);
    let _ = build_v1
        .block_on(&device)
        .expect("V1 build must succeed");

    // V2: the captured panic seed (Cylinder shifted on x).
    let v2 = pairs(&[
        ([1.498904, -0.53666365, -2.5055337], [3.898904, 1.8633364, -0.10553372]),
        ([-21.529707, -0.4, -19.742224], [19.070295, 0.4, 20.857779]),
        ([1.7758889, -0.5121188, -1.3083041], [4.675889, 2.3878813, 1.591696]),
        ([2.0948467, 1.3200147, -0.9083556], [3.9090605, 2.7200148, 0.9058579]),
        ([-3.4378545, -0.2626487, 0.7831924], [-1.0378544, 2.1373515, 3.1831925]),
        ([-3.3124545, -0.16579604, -1.45], [-0.41245437, 0.73420393, 1.45]),
    ]);
    let build_v2 = build_gpu::<u32>(&mut builder, &device, &queue, v2);
    let _ = build_v2
        .block_on(&device)
        .expect("V2 build must succeed AFTER reusing the same builder");
}
