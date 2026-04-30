//! `cfg(debug_assertions)` infrastructure for dumping the input seed
//! and post-build topology when [`super::full::BvhGpuBuild::poll`]'s
//! AABB-convergence invariant trips. The dump path is the operator's
//! handoff into the regression suite (see
//! [`super::aabb_convergence_tests`] for the pinned seeds).
//!
//! Free functions instead of methods so the build path's struct stays
//! lean — the dump only ever runs on the panic boundary.

use crate::aabb::Aabb;
use crate::node::BvhNode;

/// Debug-only AABB convergence check. In release builds the caller
/// passes `None` for `done_staging` and this is a no-op. In debug,
/// reads the staged copy of the LBVH `done[]` array and panics if
/// any internal node has `done[i] == 0` — that means the AABB
/// propagation iteration count was insufficient, the resulting
/// AABBs are silently wrong, and the caller is about to consume a
/// corrupt BVH.
pub(super) fn check_aabb_convergence_in_debug(
    n: u32,
    done_staging: Option<&wgpu::Buffer>,
    nodes_staging: &wgpu::Buffer,
    debug_input_aabbs: Option<&[Aabb]>,
) {
    let Some(staging) = done_staging else {
        return;
    };
    if n < 2 {
        return;
    }
    let n_internals = (n - 1) as usize;
    let bytes = n_internals * 4;
    let slice = staging.slice(..bytes as u64);
    let data = slice.get_mapped_range();
    let dones: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data)
        .iter()
        .copied()
        .take(n_internals)
        .collect();
    drop(data);
    staging.unmap();
    let unfinished = dones.iter().position(|&d| d == 0u32);
    if let Some(idx) = unfinished {
        let iters = crate::gpu::lbvh::aabb_iterations(n);
        let topo = read_internal_links(nodes_staging, n, n_internals);
        let dump_note =
            dump_seed_for_repro(n, debug_input_aabbs, idx, iters, &dones, &topo);
        panic!(
            "AABB iteration slack insufficient for N={n} (depth exceeded 2·log_n+4 — \
             internal node {idx} of {n_internals} unfinished after {iters} iterations). \
             done={dones:?}, topology={topo:?}. {dump_note}",
        );
    }
}

/// Read back the `(left, right_or_count)` pair for every internal
/// from a still-mapped nodes staging buffer. Best-effort — the caller
/// passes the staging slice that the convergence check has already
/// borrowed; we do not unmap because the main poll path consumes the
/// same buffer right after the convergence check, and unmapping here
/// would crash the (already-doomed) panic path.
fn read_internal_links(
    nodes_staging: &wgpu::Buffer,
    n: u32,
    n_internals: usize,
) -> Vec<(u32, u32)> {
    let total = (2 * n - 1) as usize;
    let bytes = total * std::mem::size_of::<BvhNode>();
    let slice = nodes_staging.slice(..bytes as u64);
    let data = slice.get_mapped_range();
    let nodes = bytemuck::cast_slice::<u8, BvhNode>(&data);
    let topo: Vec<(u32, u32)> = nodes
        .iter()
        .take(n_internals)
        .map(|node| (node.left, node.right_or_count))
        .collect();
    drop(data);
    topo
}

/// Serialise the captured input AABBs + topology to
/// `/tmp/lbvh_panic_seed.ron` and return a short message the panic
/// line embeds. Best-effort — every failure mode logs and returns a
/// textual fallback so the panic still surfaces the original
/// convergence diagnostic.
fn dump_seed_for_repro(
    n: u32,
    aabbs: Option<&[Aabb]>,
    unfinished_idx: usize,
    iters: u32,
    dones: &[u32],
    topo: &[(u32, u32)],
) -> String {
    let Some(aabbs) = aabbs else {
        return "No seed captured (debug_input_aabbs is None — should never happen in \
                debug). File an issue against the LBVH builder."
            .to_owned();
    };

    let path = "/tmp/lbvh_panic_seed.ron";
    let mut body = String::new();
    body.push_str("// LBVH AABB convergence failure seed — see issue #333.\n");
    body.push_str(&format!(
        "// n = {n}, unfinished_internal = {unfinished_idx}, iterations = {iters}\n"
    ));
    body.push_str(&format!("// done[] = {dones:?}\n"));
    body.push_str(&format!(
        "// internal_links (left, right_or_count) = {topo:?}\n\n"
    ));
    body.push_str("LbvhPanicSeed(\n");
    body.push_str(&format!("    n: {n},\n"));
    body.push_str(&format!("    unfinished_internal: {unfinished_idx},\n"));
    body.push_str(&format!("    iterations: {iters},\n"));
    body.push_str(&format!("    done_post_propagation: {dones:?},\n"));
    body.push_str(&format!("    internal_links: {topo:?},\n"));
    body.push_str("    aabbs: [\n");
    for a in aabbs {
        body.push_str(&format!(
            "        Aabb(min: ({:?}, {:?}, {:?}), max: ({:?}, {:?}, {:?})),\n",
            a.min.x, a.min.y, a.min.z, a.max.x, a.max.y, a.max.z,
        ));
    }
    body.push_str("    ],\n");
    body.push_str(")\n");

    match std::fs::write(path, &body) {
        Ok(()) => format!(
            "Input seed dumped to {path} ({} entries). Re-run with this seed via \
             the regression test in `crates/ome_bvh/src/gpu/build/aabb_convergence_tests.rs`.",
            aabbs.len()
        ),
        Err(e) => format!(
            "Failed to dump seed to {path}: {e}. Inputs (n={n}): {aabbs:?}"
        ),
    }
}
