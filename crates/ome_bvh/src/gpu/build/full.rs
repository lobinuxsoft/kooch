//! [`BvhGpuBuild<T>`] + [`build_gpu`] — full Karras GPU LBVH path.
//!
//! Chains morton + onesweep sort + Karras (leaves + internal + AABB
//! propagation) on a single command encoder, submits, and returns a
//! poll-driven handle. The `n == 0` and `n == 1` branches are made
//! visible at the call site so the orchestrator's edge-case guards
//! stay reviewable.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::gpu::builder::BvhGpuBuilder;
use crate::node::BvhNode;

use super::error::BvhBuildError;
use super::lifecycle::{GpuBvhHandle, MapState};

/// Result of a successfully resolved [`BvhGpuBuild::poll`]. Carries
/// the byte-identical CPU mirror of the GPU build alongside the
/// `sorted_indices` permutation the readback already paid for. The
/// permutation feeds [`Bvh::refit_in_place`] so a CPU consumer
/// (physics broadphase, debug tooling, ...) can stay in sync with
/// subsequent refits over the same topology without a fresh nodes
/// readback.
pub struct BvhGpuBuildResult<T: Copy> {
    pub bvh: Bvh<T>,
    /// `sorted_indices[k]` = original-input position of the leaf at
    /// sorted position `k` (i.e. `nodes[(N-1) + k]`). Empty for the
    /// `n == 0` build path.
    pub sorted_indices: Vec<u32>,
}

/// Handle to a GPU LBVH build in flight. Drive with
/// [`Self::poll`] (frame-loop friendly) or [`Self::block_on`] (tests +
/// tooling, gated behind `cfg(any(test, feature = "block_on"))`).
///
/// **Single-shot consumable**: once [`Self::poll`] returns `Some(_)`,
/// further polls return `None`. Discard the handle after consumption.
pub struct BvhGpuBuild<T: Copy> {
    n: u32,
    /// `None` for the trivial `n == 0` case (no submission to wait on);
    /// `Some(idx)` once the build has been submitted.
    submission_index: Option<wgpu::SubmissionIndex>,
    /// Staging buffer holding the `2N-1` `BvhNode`s once mapped.
    nodes_staging: wgpu::Buffer,
    /// Staging buffer holding the `N` sorted indices once mapped.
    /// Maps `sorted_position → original_payload_index`.
    indices_staging: wgpu::Buffer,
    /// Refcounted clone of the builder's `lbvh_buffers.nodes_buffer`.
    /// Exposed via [`Self::gpu_handle`] for downstream traversal
    /// kernels without forcing a readback.
    nodes_buffer: wgpu::Buffer,
    map_state: Arc<MapState>,
    /// Original-order payloads, indexed by the sorted indices once
    /// the readback completes to produce `Bvh::leaves`.
    payloads: Vec<T>,
    /// Set true after [`Self::poll`] returns `Some(_)` so subsequent
    /// polls don't try to unmap an already-unmapped buffer.
    consumed: bool,
    /// **Debug-only** staging copy of the LBVH `done` array. Populated
    /// only when `cfg!(debug_assertions) && n >= 2`; `None` in release
    /// or for trivial sizes. When `Some`, [`Self::poll`] reads it once
    /// the build resolves and panics if any internal node has
    /// `done[i] == 0`. See `AABB_ITERATION_SLACK` in `gpu/lbvh.rs`.
    done_staging: Option<wgpu::Buffer>,
    /// **Debug-only** copy of the input AABBs in original order.
    /// Captured at `build_gpu` entry so the convergence-failure
    /// dump (#333) records the exact seed the LBVH builder
    /// received. `Some` mirrors `done_staging`'s gating.
    debug_input_aabbs: Option<Vec<Aabb>>,
}

impl<T: Copy> BvhGpuBuild<T> {
    /// Non-blocking check. Returns:
    ///
    /// - `None` while either staging buffer is still in flight.
    /// - `Some(Ok(BvhGpuBuildResult))` once both buffers are mapped —
    ///   the `bvh` field is byte-identical to `Bvh::build(items)` and
    ///   `sorted_indices` is the permutation the readback already
    ///   produced (re-used by [`Bvh::refit_in_place`] downstream).
    /// - `Some(Err(_))` if the device was lost or a buffer map failed.
    ///
    /// Caller is expected to call `device.poll(PollType::Poll)` once
    /// per frame so wgpu invokes the `map_async` callbacks; this
    /// function only reads the resulting atomic flags.
    pub fn poll(
        &mut self,
        _device: &wgpu::Device,
    ) -> Option<Result<BvhGpuBuildResult<T>, BvhBuildError>> {
        if self.consumed {
            return None;
        }
        if self.submission_index.is_none() {
            // n == 0 path — empty BVH resolves immediately.
            self.consumed = true;
            return Some(Ok(BvhGpuBuildResult {
                bvh: Bvh::empty(),
                sorted_indices: Vec::new(),
            }));
        }
        if self.map_state.nodes_err.load(Ordering::Acquire)
            || self.map_state.indices_err.load(Ordering::Acquire)
            || self.map_state.done_err.load(Ordering::Acquire)
        {
            self.consumed = true;
            return Some(Err(BvhBuildError::BufferMapFailed));
        }
        if !self.map_state.nodes_done.load(Ordering::Acquire)
            || !self.map_state.indices_done.load(Ordering::Acquire)
        {
            return None;
        }
        // Debug-only invariant check: wait for the done staging map
        // (same submission as nodes/indices, so it lands during the
        // same `device.poll`) and verify every internal converged.
        if self.done_staging.is_some() && !self.map_state.done_done.load(Ordering::Acquire) {
            return None;
        }

        // Both staging buffers are mapped — assemble the Bvh<T>.
        self.consumed = true;
        self.check_aabb_convergence_in_debug();
        let total_nodes = (2 * self.n - 1) as usize;
        let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>();
        let nodes: Vec<BvhNode> = {
            let slice = self.nodes_staging.slice(..nodes_bytes as u64);
            let data = slice.get_mapped_range();
            let v = bytemuck::cast_slice::<u8, BvhNode>(&data).to_vec();
            drop(data);
            v
        };
        self.nodes_staging.unmap();

        let sorted_indices: Vec<u32> = {
            let slice = self.indices_staging.slice(..(self.n as u64) * 4);
            let data = slice.get_mapped_range();
            let v = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
            drop(data);
            v
        };
        self.indices_staging.unmap();

        // Permute payloads via sorted_indices. CPU work is O(n) +
        // O(n) clones — trivial vs the GPU build cost, and only paid
        // by callers that actually want the CPU-side `Bvh<T>` (the
        // GPU-resident path via `gpu_handle` skips this entirely).
        let leaves: Vec<T> = sorted_indices
            .iter()
            .map(|&i| self.payloads[i as usize])
            .collect();
        Some(Ok(BvhGpuBuildResult {
            bvh: Bvh { nodes, leaves },
            sorted_indices,
        }))
    }

    /// Debug-only AABB convergence check — see
    /// [`super::seed_dump::check_aabb_convergence_in_debug`].
    fn check_aabb_convergence_in_debug(&self) {
        super::seed_dump::check_aabb_convergence_in_debug(
            self.n,
            self.done_staging.as_ref(),
            &self.nodes_staging,
            self.debug_input_aabbs.as_deref(),
        );
    }

    /// GPU-resident view of the (in-flight or completed) tree.
    /// **First call blocks** on the build's submission so the buffer
    /// contents are guaranteed valid; subsequent calls return
    /// immediately. PR-4 / PR-5 traversal kernels bind
    /// `handle.nodes_buffer` directly — no readback path involved.
    pub fn gpu_handle(&self, device: &wgpu::Device) -> GpuBvhHandle<'_> {
        if let Some(idx) = self.submission_index.clone() {
            // `PollType::Wait { submission_index: Some(idx), .. }` is
            // a no-op once that submission has already completed.
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: Some(idx),
                timeout: None,
            });
        }
        GpuBvhHandle {
            nodes_buffer: &self.nodes_buffer,
            n: self.n,
        }
    }

    /// Test + tooling helper: drive the device polling loop until the
    /// build resolves, then return the result. **Never** call from a
    /// frame loop — `device.poll(Wait)` blocks the calling thread.
    #[cfg(any(test, feature = "block_on"))]
    pub fn block_on(
        mut self,
        device: &wgpu::Device,
    ) -> Result<BvhGpuBuildResult<T>, BvhBuildError> {
        let Some(idx) = self.submission_index.clone() else {
            // n == 0 short-circuit.
            return self
                .poll(device)
                .expect("empty build resolves on first poll");
        };
        loop {
            match device.poll(wgpu::PollType::Wait {
                submission_index: Some(idx.clone()),
                timeout: Some(std::time::Duration::from_secs(30)),
            }) {
                Ok(_) => {}
                Err(_) => return Err(BvhBuildError::DeviceLost),
            }
            if let Some(result) = self.poll(device) {
                return result;
            }
            // wgpu invokes map_async callbacks during device.poll, so
            // this loop should resolve in 1-2 iterations. Defensive
            // continuation absorbs any extra poll round-trip needed
            // for the second buffer's callback to fire.
        }
    }
}

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

/// Construct a placeholder `BvhGpuBuild` for the `n == 0` case. No GPU
/// dispatches and no submission — `submission_index = None` makes
/// `BvhGpuBuild::poll` return `Some(Ok(Bvh::empty()))` immediately.
/// Staging buffers are minimal placeholders that are never mapped.
fn empty_build<T: Copy>(builder: &BvhGpuBuilder, device: &wgpu::Device) -> BvhGpuBuild<T> {
    let placeholder = |label: &str| -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    BvhGpuBuild {
        n: 0,
        submission_index: None,
        nodes_staging: placeholder("ome_bvh::build_gpu_empty_nodes_staging"),
        indices_staging: placeholder("ome_bvh::build_gpu_empty_indices_staging"),
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state: Arc::new(MapState::default()),
        payloads: Vec::new(),
        consumed: false,
        done_staging: None,
        debug_input_aabbs: None,
    }
}
