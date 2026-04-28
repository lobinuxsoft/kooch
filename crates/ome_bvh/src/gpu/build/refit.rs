//! [`BvhGpuRefit`] + [`refit_gpu`] — topology-preserving fast path.
//!
//! Rewrites only the leaves and propagates internal AABBs over an
//! already-built topology. Skips morton encoding, the onesweep sort,
//! and Karras' internal-node construction entirely. Uses a 4-byte
//! fence staging copy as the "submission completed" signal so the
//! caller never has to read back the full `(2N-1) * 32 B` nodes
//! buffer per frame.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::aabb::Aabb;
use crate::gpu::builder::BvhGpuBuilder;

use super::error::BvhBuildError;
use super::lifecycle::{GpuBvhHandle, MapState};

/// Handle to an in-flight GPU LBVH **refit**. Symmetrical to
/// [`super::BvhGpuBuild`] but on the **fast path**: rewrites only
/// the leaves + propagates internal AABBs over an already-built
/// topology.
///
/// Caller invariants (the orchestrator
/// [`crate::SharedBvhState::kick_refit`] enforces them):
///
/// 1. The previous build's outputs are still resident in the
///    builder's scratch buffers (`builder.nodes_buffer`,
///    `builder.sorted_indices_buffer`). No intermediate failed build
///    or partial reset has clobbered them.
/// 2. `items.len()` equals the previous build's `n`.
/// 3. `items[i].0` (payload identity) is at the **same array
///    position** as in the previous build. Only `items[i].1` (the
///    AABB) may have changed.
///
/// Violating (2) or (3) does not panic — it produces wrong-but-
/// plausible AABBs (the leaves get re-AABB'd through the previous
/// permutation; topological mismatch is silent). Always go through
/// the orchestrator.
pub struct BvhGpuRefit {
    n: u32,
    submission_index: Option<wgpu::SubmissionIndex>,
    /// 4-byte dummy buffer used purely as a fence: `map_async` fires
    /// once every preceding compute pass on the same encoder has
    /// completed, so the renderer learns "the refit is done" without
    /// paying for a `(2N-1) * 32 B` nodes readback. Only exists for
    /// `n >= 1`.
    fence_staging: wgpu::Buffer,
    /// Refcounted clone of the builder's `lbvh_buffers.nodes_buffer`,
    /// kept alive so [`Self::gpu_handle`] can hand out a stable
    /// reference until the caller drops the handle.
    nodes_buffer: wgpu::Buffer,
    map_state: Arc<MapState>,
    consumed: bool,
    /// Same debug-only convergence check as the build path.
    done_staging: Option<wgpu::Buffer>,
}

impl BvhGpuRefit {
    /// Non-blocking check. Returns `Some(Ok(()))` once the refit's
    /// submission completes, `None` while in flight, `Some(Err(_))`
    /// on device-loss / map failure.
    pub fn poll(&mut self, _device: &wgpu::Device) -> Option<Result<(), BvhBuildError>> {
        if self.consumed {
            return None;
        }
        if self.submission_index.is_none() {
            // n == 0 trivially resolved.
            self.consumed = true;
            return Some(Ok(()));
        }
        if self.map_state.nodes_err.load(Ordering::Acquire)
            || self.map_state.done_err.load(Ordering::Acquire)
        {
            self.consumed = true;
            return Some(Err(BvhBuildError::BufferMapFailed));
        }
        if !self.map_state.nodes_done.load(Ordering::Acquire) {
            return None;
        }
        if self.done_staging.is_some() && !self.map_state.done_done.load(Ordering::Acquire) {
            return None;
        }
        self.consumed = true;
        self.check_aabb_convergence_in_debug();
        self.fence_staging.unmap();
        Some(Ok(()))
    }

    /// GPU-resident view of the (in-flight or completed) refit'd tree.
    /// **First call blocks** on the refit's submission so the buffer
    /// contents are guaranteed valid; subsequent calls return
    /// immediately. Same contract as [`super::BvhGpuBuild::gpu_handle`].
    pub fn gpu_handle(&self, device: &wgpu::Device) -> GpuBvhHandle<'_> {
        if let Some(idx) = self.submission_index.clone() {
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
    /// refit resolves. **Never** call from a frame loop.
    #[cfg(any(test, feature = "block_on"))]
    pub fn block_on(mut self, device: &wgpu::Device) -> Result<(), BvhBuildError> {
        let Some(idx) = self.submission_index.clone() else {
            return self
                .poll(device)
                .expect("empty refit resolves on first poll");
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
        }
    }

    fn check_aabb_convergence_in_debug(&self) {
        let Some(ref staging) = self.done_staging else {
            return;
        };
        if self.n < 2 {
            return;
        }
        let n_internals = (self.n - 1) as usize;
        let bytes = n_internals * 4;
        let slice = staging.slice(..bytes as u64);
        let data = slice.get_mapped_range();
        let dones = bytemuck::cast_slice::<u8, u32>(&data);
        let unfinished = dones
            .iter()
            .take(n_internals)
            .position(|&d| d == 0u32);
        drop(data);
        staging.unmap();
        if let Some(idx) = unfinished {
            let iters = crate::gpu::lbvh::aabb_iterations(self.n);
            panic!(
                "AABB iteration slack insufficient during REFIT for N={} (depth exceeded \
                 2·log_n+4 — internal node {idx} of {n_internals} unfinished after {iters} \
                 iterations). Topology was preserved from the previous build, so this means \
                 the original tree was already adversarial and slack ran out only on refit.",
                self.n
            );
        }
    }
}

/// Free-function form of `Bvh::refit_gpu`. Re-exported on `Bvh<T>` in
/// `bvh.rs` so callers write `Bvh::<u32>::refit_gpu(&mut builder, ...)`.
///
/// **Caller invariants** — see [`BvhGpuRefit`] for details. Briefly:
/// items count and order must match the immediately-preceding build;
/// only AABBs may change. The orchestrator
/// [`crate::SharedBvhState::kick_refit`] enforces these.
pub fn refit_gpu<T: Copy + bytemuck::Pod>(
    builder: &mut BvhGpuBuilder,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    items: Vec<(T, Aabb)>,
) -> BvhGpuRefit {
    let n = items.len() as u32;

    if n == 0 {
        return empty_refit(builder, device);
    }

    builder.ensure_capacity(device, n as u64);

    let (_payloads, aabbs): (Vec<T>, Vec<Aabb>) = items.into_iter().unzip();

    // Upload the new AABBs into the same `aabbs_buffer` the leaves
    // pass reads through the existing `sorted_indices` permutation.
    let gpu_aabbs: Vec<crate::gpu::types::GpuAabb> =
        aabbs.iter().copied().map(crate::gpu::types::GpuAabb::from).collect();
    queue.write_buffer(
        &builder.aabbs_buffer,
        0,
        bytemuck::cast_slice(&gpu_aabbs),
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::refit_gpu_encoder"),
    });

    // Reset internals' `done` flags so the propagation pass re-merges
    // every internal from its (newly-AABB'd) children. Leaves stay
    // pinned at done=1 by the leaves-rewrite pass below.
    if n >= 2 {
        let internals_bytes = ((n - 1) as u64) * 4;
        encoder.clear_buffer(
            &builder.lbvh_buffers.done_buffer,
            0,
            Some(internals_bytes),
        );
    }

    // Pass 1: rewrite leaves with new AABBs through the previous
    // build's `sort_buffers.values_a` (sorted_indices). Topology
    // preserved — only `nodes[(N-1)..(2N-1)).aabb_*` change.
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

    // Pass 3: AABB propagation only. `karras_internal` (pass 2) is
    // skipped — the internal nodes' `left` / `right_or_count` were
    // written by the previous build and remain valid.
    if n >= 2 {
        crate::gpu::lbvh::dispatch_lbvh_aabb_only_into(
            device,
            queue,
            &mut encoder,
            &builder.lbvh_pipelines,
            &builder.lbvh_buffers,
            n,
        );
    }

    // Fence staging: a 4-byte dummy whose `map_async` callback fires
    // once every preceding compute pass on this encoder has
    // completed. Tiny — copying 4 bytes per refit costs nothing
    // compared to the `(2N-1) * 32 B` a full nodes readback would
    // burn each frame.
    let fence_staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::refit_gpu_fence_staging"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // Source the fence copy from `nodes_buffer` — its first 4 bytes
    // are part of `nodes[0].aabb_min.x` and we don't read them. The
    // nodes buffer is the one downstream consumers wait on, so the
    // copy is queued strictly after the propagation passes; that's
    // the only ordering property the fence needs. `config_buffer`
    // lacks `COPY_SRC` and would require widening usage flags for
    // no good reason.
    encoder.copy_buffer_to_buffer(
        &builder.lbvh_buffers.nodes_buffer,
        0,
        &fence_staging,
        0,
        4,
    );

    // Debug-only convergence check (same shape as `build_gpu`).
    let done_staging = if cfg!(debug_assertions) && n >= 2 {
        let n_internals = (n - 1) as u64;
        let bytes = n_internals * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::refit_gpu_done_staging_debug"),
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

    let map_state = Arc::new(MapState::default());
    {
        let st = map_state.clone();
        fence_staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| match res {
                Ok(()) => st.nodes_done.store(true, Ordering::Release),
                Err(_) => st.nodes_err.store(true, Ordering::Release),
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

    BvhGpuRefit {
        n,
        submission_index: Some(submission_index),
        fence_staging,
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state,
        consumed: false,
        done_staging,
    }
}

/// Construct a placeholder `BvhGpuRefit` for the `n == 0` case. No
/// dispatches and no submission — `BvhGpuRefit::poll` resolves on
/// the first call.
fn empty_refit(builder: &BvhGpuBuilder, device: &wgpu::Device) -> BvhGpuRefit {
    let placeholder = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::refit_gpu_empty_fence_staging"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    BvhGpuRefit {
        n: 0,
        submission_index: None,
        fence_staging: placeholder,
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state: Arc::new(MapState::default()),
        consumed: false,
        done_staging: None,
    }
}
