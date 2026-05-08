use super::buffers::LbvhBuffers;
use super::pipelines::LbvhPipelines;
use super::{LbvhConfig, aabb_iterations};
use crate::gpu::karras_common::KARRAS_WORKGROUP_SIZE;

/// Pass 1 of the Karras build: write the N leaves into
/// `nodes[(N-1)..(2N-1))` and set `done[leaf_idx] = 1`. Safe for any
/// `n >= 1`. The orchestrator calls this unconditionally and then
/// branches on `n > 1` for the internal + AABB passes — keeping the
/// control flow visible at the call site (and avoiding the
/// `workgroup_count = 0` UB risk that some backends exhibit when
/// dispatched on the internal pass with `n == 1`).
pub fn dispatch_lbvh_leaves_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    original_aabbs: &wgpu::Buffer,
    sorted_indices: &wgpu::Buffer,
    n: u32,
) {
    if n == 0 {
        return;
    }

    // The config uniform is shared with the internal + aabb passes; we
    // write it once here so subsequent passes pick up the same `n`.
    let cfg = LbvhConfig {
        n,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
    };
    queue.write_buffer(&buffers.config_buffer, 0, bytemuck::bytes_of(&cfg));

    let leaves_workgroups = n.div_ceil(KARRAS_WORKGROUP_SIZE);

    let leaves_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_leaves_bg"),
        layout: &pipelines.leaves_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: original_aabbs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: sorted_indices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ome_bvh::karras_leaves_pass"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipelines.leaves_pipeline);
    pass.set_bind_group(0, &leaves_bg, &[]);
    pass.dispatch_workgroups(leaves_workgroups.max(1), 1, 1);
}

/// Passes 2+3 of the Karras build: parallel internal-node construction
/// followed by `⌈log₂ n⌉ + slack` AABB propagation dispatches.
/// **Requires `n >= 2`** — the caller must guard against the trivial
/// 1-leaf case (where there are no internals to build).
pub fn dispatch_lbvh_internal_and_aabb_into(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    sorted_morton: &wgpu::Buffer,
    n: u32,
) {
    debug_assert!(
        n >= 2,
        "dispatch_lbvh_internal_and_aabb_into requires n >= 2 — \
         orchestrator must skip this for n in {{0, 1}}"
    );

    let internal_workgroups = (n - 1).div_ceil(KARRAS_WORKGROUP_SIZE);

    let internal_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_internal_bg"),
        layout: &pipelines.internal_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: sorted_morton.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.parents_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::karras_internal_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.internal_pipeline);
        pass.set_bind_group(0, &internal_bg, &[]);
        pass.dispatch_workgroups(internal_workgroups.max(1), 1, 1);
    }

    // -- pass 3: aabb propagation, looped one tree level per dispatch --
    let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_aabb_bg"),
        layout: &pipelines.aabb_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    let iterations = aabb_iterations(n);
    for iter in 0..iterations {
        let label = format!("ome_bvh::karras_aabb_pass_{iter}");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.aabb_pipeline);
        pass.set_bind_group(0, &aabb_bg, &[]);
        pass.dispatch_workgroups(internal_workgroups.max(1), 1, 1);
    }
}

/// AABB propagation only (pass 3) — the refit fast path.
///
/// Reuses the same compute pipeline as the full build's pass 3 but
/// skips the internal-node construction (pass 2). Caller invariants:
///
/// 1. `buffers.nodes_buffer` already holds a previously-built BVH's
///    topology (left / right_or_count of every internal node). The
///    refit preserves topology — only AABBs change.
/// 2. `buffers.done_buffer[0..n-1]` has been reset to `0` (caller
///    typically uses `encoder.clear_buffer`); leaves' done bits are
///    re-set to 1 by an immediately-preceding leaves-rewrite pass.
/// 3. The leaves at `nodes[(n-1)..(2n-1))` already hold the new
///    AABBs (caller dispatched [`dispatch_lbvh_leaves_into`] with the
///    fresh `original_aabbs` first).
///
/// **Requires `n >= 2`** — the caller skips this for `n in {0, 1}`
/// (no internals to refit).
pub fn dispatch_lbvh_aabb_only_into(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    n: u32,
) {
    debug_assert!(
        n >= 2,
        "dispatch_lbvh_aabb_only_into requires n >= 2 — \
         orchestrator must skip this for n in {{0, 1}}"
    );

    let internal_workgroups = (n - 1).div_ceil(KARRAS_WORKGROUP_SIZE);
    let aabb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_bvh::karras_aabb_bg_refit"),
        layout: &pipelines.aabb_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.done_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.config_buffer.as_entire_binding(),
            },
        ],
    });
    let iterations = aabb_iterations(n);
    for iter in 0..iterations {
        let label = format!("ome_bvh::karras_aabb_refit_pass_{iter}");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(&label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipelines.aabb_pipeline);
        pass.set_bind_group(0, &aabb_bg, &[]);
        pass.dispatch_workgroups(internal_workgroups.max(1), 1, 1);
    }
}

/// Convenience wrapper that dispatches the full Karras build (leaves +
/// internal + aabb) for any `n >= 0`. Used by the existing per-stage
/// integration tests; production code in [`crate::gpu::build`] calls
/// the two halves explicitly so the `n == 1` branch is visible.
///
/// Inputs (caller-owned buffers; not modified by this function):
/// - `original_aabbs`: storage buffer of [`crate::gpu::types::GpuAabb`]
///   in the *original* (pre-sort) order. Used by `write_leaves` via
///   the indirection through `sorted_indices`.
/// - `sorted_morton`: storage buffer of `u32` Morton codes after the
///   onesweep sort (length `n`).
/// - `sorted_indices`: storage buffer of `u32` payload indices after
///   the onesweep sort (length `n`). `sorted_indices[k]` is the
///   *original* index of the item at Morton-sorted position `k`.
///
/// Output: `buffers.nodes_buffer` populated with `2n-1` BvhNodes
/// byte-identical to `Bvh::build(items).nodes`.
pub fn dispatch_lbvh_build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &LbvhPipelines,
    buffers: &LbvhBuffers,
    original_aabbs: &wgpu::Buffer,
    sorted_morton: &wgpu::Buffer,
    sorted_indices: &wgpu::Buffer,
    n: u32,
) {
    if n == 0 {
        return;
    }
    dispatch_lbvh_leaves_into(
        device,
        queue,
        encoder,
        pipelines,
        buffers,
        original_aabbs,
        sorted_indices,
        n,
    );
    if n >= 2 {
        dispatch_lbvh_internal_and_aabb_into(
            device,
            queue,
            encoder,
            pipelines,
            buffers,
            sorted_morton,
            n,
        );
    }
}
