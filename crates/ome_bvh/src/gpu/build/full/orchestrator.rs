//! [`build_gpu`] — free-function orchestrator that chains morton +
//! onesweep sort + Karras LBVH on a single command encoder, submits,
//! and arms the staging readbacks behind the [`super::BvhGpuBuild`]
//! handle.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::aabb::Aabb;
use crate::gpu::builder::BvhGpuBuilder;
use crate::node::BvhNode;

use super::super::lifecycle::MapState;
use super::build::BvhGpuBuild;
use super::empty::empty_build;

/// Free-function form of `Bvh::build_gpu`. Re-exported on `Bvh<T>` in
/// `bvh.rs` so callers write `Bvh::<u32>::build_gpu(&mut builder, ...)`.
pub fn build_gpu<T: Copy + bytemuck::Pod>(
    builder: &mut BvhGpuBuilder,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    items: Vec<(T, Aabb)>,
) -> BvhGpuBuild<T> {
    let n = items.len() as u32;

    // Trivial zero-item case: handle resolves to `Bvh::empty()` on
    // first poll without dispatching anything. We still construct a
    // dummy submission_index + staging buffers because the struct
    // fields are non-Option; gpu_handle short-circuits on `empty`.
    if n == 0 {
        return empty_build(builder, device);
    }

    builder.ensure_capacity(device, n as u64);

    // Split ownership: `payloads` keeps the original-order Ts for
    // post-readback permutation; `aabbs` is consumed by the morton
    // upload.
    let (payloads, aabbs): (Vec<T>, Vec<Aabb>) = items.into_iter().unzip();

    // Snapshot the input AABBs in debug for #333's seed-dump path.
    // The builder's other passes consume `aabbs` by reference, so we
    // can clone before the morton dispatch without affecting the
    // build. Vec<Aabb> at N=6 costs ~144 bytes — negligible.
    let debug_input_aabbs: Option<Vec<Aabb>> = if cfg!(debug_assertions) {
        Some(aabbs.clone())
    } else {
        None
    };

    // Initialise the sort's `values_a` with sequential indices
    // [0, n). Onesweep permutes these in lockstep with the keys, so
    // after the sort `values_a[k] = original_payload_index_at_sorted_position_k`.
    // queue.write_buffer is staged before the encoder commands run
    // within a submission.
    let initial_values: Vec<u32> = (0..n).collect();
    queue.write_buffer(
        &builder.sort_buffers.values_a,
        0,
        bytemuck::cast_slice(&initial_values),
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::build_gpu_encoder"),
    });

    // 1. Morton: `aabbs_buffer + scene_bounds → morton_codes_buffer`.
    builder.dispatch_morton_into(device, queue, &mut encoder, &aabbs);

    // Bridge morton_codes → keys_a so the sort can consume from its
    // own ping-pong buffer pair. encoder-ordered copy guarantees the
    // morton compute pass completes before the histogram reads keys_a.
    encoder.copy_buffer_to_buffer(
        &builder.morton_codes_buffer,
        0,
        &builder.sort_buffers.keys_a,
        0,
        (n as u64) * 4,
    );

    // 2. Sort: 4-pass onesweep radix. Sorted keys land in keys_a (even
    // pass count); permuted indices land in values_a. For n == 1 the
    // sort is a no-op (one element is trivially sorted) and we skip
    // the dispatch entirely — onesweep's partition descriptor logic
    // assumes at least one full tile and gets confused at n=1.
    if n >= 2 {
        crate::gpu::sort::dispatch_sort_into(
            device,
            queue,
            &mut encoder,
            &builder.sort_pipelines,
            &builder.sort_buffers,
            n,
        );
    }

    // 3a. LBVH leaves — always (any n >= 1). Reads
    // `original_aabbs[sorted_indices[k]]` so it works whether the sort
    // ran or not (n == 1 case: sorted_indices[0] = 0 from the initial
    // upload, lookup hits the only leaf).
    crate::gpu::lbvh::dispatch_lbvh_leaves_into(
        device,
        queue,
        &mut encoder,
        &builder.lbvh_pipelines,
        &builder.lbvh_buffers,
        &builder.aabbs_buffer,
        &builder.sort_buffers.values_a,
        n,
    );

    // 3b. LBVH internal + AABB propagation — only when there's at
    // least one internal node (n >= 2). For n == 1, the single leaf
    // at `nodes[0]` IS the root; no further dispatches needed.
    if n >= 2 {
        crate::gpu::lbvh::dispatch_lbvh_internal_and_aabb_into(
            device,
            queue,
            &mut encoder,
            &builder.lbvh_pipelines,
            &builder.lbvh_buffers,
            &builder.sort_buffers.keys_a,
            n,
        );
    }

    // Stage the outputs. `2N-1` BvhNodes + `N` sorted indices are
    // copied to MAP_READ buffers; map_async callbacks below will set
    // the AtomicBool flags read by `BvhGpuBuild::poll`.
    let total_nodes = (2 * n - 1) as u64;
    let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>() as u64;
    let nodes_staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::build_gpu_nodes_staging"),
        size: nodes_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(
        &builder.lbvh_buffers.nodes_buffer,
        0,
        &nodes_staging,
        0,
        nodes_bytes,
    );

    let indices_bytes = (n as u64) * 4;
    let indices_staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::build_gpu_indices_staging"),
        size: indices_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(
        &builder.sort_buffers.values_a,
        0,
        &indices_staging,
        0,
        indices_bytes,
    );

    // Debug-only: stage a copy of the LBVH `done` array so
    // `BvhGpuBuild::poll` can verify every internal node converged
    // (i.e. `done[i] == 1` for all `i in 0..n-1`). Catches the case
    // where the AABB iteration count was insufficient — silently
    // wrong AABBs are a planet-scale-grade footgun. Skipped in
    // release for zero overhead.
    let done_staging = if cfg!(debug_assertions) && n >= 2 {
        let n_internals = (n - 1) as u64;
        let bytes = n_internals * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::build_gpu_done_staging_debug"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(
            &builder.lbvh_buffers.done_buffer,
            0,
            &staging,
            0,
            bytes,
        );
        Some(staging)
    } else {
        None
    };

    let submission_index = queue.submit(std::iter::once(encoder.finish()));

    // Arm map_async on the staging buffers. Callbacks fire from
    // inside `device.poll(...)` invocations on the calling thread.
    let map_state = Arc::new(MapState::default());
    {
        let st = map_state.clone();
        nodes_staging.slice(..).map_async(wgpu::MapMode::Read, move |res| match res {
            Ok(()) => st.nodes_done.store(true, Ordering::Release),
            Err(_) => st.nodes_err.store(true, Ordering::Release),
        });
    }
    {
        let st = map_state.clone();
        indices_staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| match res {
                Ok(()) => st.indices_done.store(true, Ordering::Release),
                Err(_) => st.indices_err.store(true, Ordering::Release),
            });
    }
    if let Some(ref staging) = done_staging {
        let st = map_state.clone();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| match res {
                Ok(()) => st.done_done.store(true, Ordering::Release),
                Err(_) => st.done_err.store(true, Ordering::Release),
            });
    }

    BvhGpuBuild {
        n,
        submission_index: Some(submission_index),
        nodes_staging,
        indices_staging,
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state,
        payloads,
        consumed: false,
        done_staging,
        debug_input_aabbs,
    }
}
