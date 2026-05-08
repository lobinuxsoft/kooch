//! [`BvhGpuBuild<T>`] — poll-driven handle for an in-flight GPU LBVH
//! build. Drives both the CPU readback path
//! ([`BvhGpuBuild::poll`] / [`BvhGpuBuild::block_on`]) and the
//! GPU-resident handoff ([`BvhGpuBuild::gpu_handle`]).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::node::BvhNode;

use super::super::error::BvhBuildError;
use super::super::lifecycle::{GpuBvhHandle, MapState};
use super::result::BvhGpuBuildResult;

/// Handle to a GPU LBVH build in flight. Drive with
/// [`Self::poll`] (frame-loop friendly) or [`Self::block_on`] (tests +
/// tooling, gated behind `cfg(any(test, feature = "block_on"))`).
///
/// **Single-shot consumable**: once [`Self::poll`] returns `Some(_)`,
/// further polls return `None`. Discard the handle after consumption.
pub struct BvhGpuBuild<T: Copy> {
    pub(super) n: u32,
    /// `None` for the trivial `n == 0` case (no submission to wait on);
    /// `Some(idx)` once the build has been submitted.
    pub(super) submission_index: Option<wgpu::SubmissionIndex>,
    /// Staging buffer holding the `2N-1` `BvhNode`s once mapped.
    pub(super) nodes_staging: wgpu::Buffer,
    /// Staging buffer holding the `N` sorted indices once mapped.
    /// Maps `sorted_position → original_payload_index`.
    pub(super) indices_staging: wgpu::Buffer,
    /// Refcounted clone of the builder's `lbvh_buffers.nodes_buffer`.
    /// Exposed via [`Self::gpu_handle`] for downstream traversal
    /// kernels without forcing a readback.
    pub(super) nodes_buffer: wgpu::Buffer,
    pub(super) map_state: Arc<MapState>,
    /// Original-order payloads, indexed by the sorted indices once
    /// the readback completes to produce `Bvh::leaves`.
    pub(super) payloads: Vec<T>,
    /// Set true after [`Self::poll`] returns `Some(_)` so subsequent
    /// polls don't try to unmap an already-unmapped buffer.
    pub(super) consumed: bool,
    /// **Debug-only** staging copy of the LBVH `done` array. Populated
    /// only when `cfg!(debug_assertions) && n >= 2`; `None` in release
    /// or for trivial sizes. When `Some`, [`Self::poll`] reads it once
    /// the build resolves and panics if any internal node has
    /// `done[i] == 0`. See `AABB_ITERATION_SLACK` in `gpu/lbvh.rs`.
    pub(super) done_staging: Option<wgpu::Buffer>,
    /// **Debug-only** copy of the input AABBs in original order.
    /// Captured at `build_gpu` entry so the convergence-failure
    /// dump (#333) records the exact seed the LBVH builder
    /// received. `Some` mirrors `done_staging`'s gating.
    pub(super) debug_input_aabbs: Option<Vec<Aabb>>,
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
    /// [`super::super::seed_dump::check_aabb_convergence_in_debug`].
    fn check_aabb_convergence_in_debug(&self) {
        super::super::seed_dump::check_aabb_convergence_in_debug(
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
