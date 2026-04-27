//! High-level GPU LBVH build orchestrator.
//!
//! [`Bvh::build_gpu`] chains morton + onesweep sort + Karras LBVH on a
//! single command encoder, submits, and returns a [`BvhGpuBuild<T>`]
//! handle. The handle is poll-driven (see
//! [`BvhGpuBuild::poll`]) so the caller can integrate it into a
//! frame loop without ever calling `block_on` from the hot path.
//!
//! Two consumption modes are supported:
//!
//! - **CPU readback** (tests, tooling, oneshot tools): the caller polls
//!   until [`BvhGpuBuild::poll`] returns `Some(Ok(Bvh<T>))`, recovering
//!   the flat `Vec<BvhNode>` and the permuted `Vec<T>` payload.
//! - **GPU-resident handoff** (production hot loop, PR-4 raymarch
//!   culling, PR-5 collision broadphase): the caller does NOT readback;
//!   instead it grabs [`BvhGpuBuild::gpu_handle`] which exposes the
//!   `nodes_buffer` + `n` for downstream traversal kernels. The first
//!   call to `gpu_handle` blocks on the build's submission to ensure
//!   the buffer contents are valid; subsequent calls are free.
//!
//! See `crates/ome_bvh/src/gpu/builder.rs` for the underlying state
//! (pipelines + reusable buffers).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::aabb::Aabb;
use crate::bvh::Bvh;
use crate::gpu::builder::BvhGpuBuilder;
use crate::node::BvhNode;

/// Errors surfaced by the GPU build pipeline. All are terminal — the
/// caller should drop the [`BvhGpuBuild`] and either retry or fail
/// loudly. Returned through [`BvhGpuBuild::poll`] /
/// [`BvhGpuBuild::block_on`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BvhBuildError {
    /// `wgpu::Device` was lost (driver crash, surface reconfigured
    /// mid-build, etc). The build's submission can't complete and the
    /// staging buffers will never resolve.
    DeviceLost,
    /// `map_async` callback reported a buffer mapping failure on either
    /// the nodes or the sorted-indices staging buffer.
    BufferMapFailed,
    /// Buffer allocation failed during `ensure_capacity` — the GPU is
    /// out of memory or the requested size exceeded
    /// `max_buffer_size`. The build was abandoned before submission.
    OutOfMemory,
}

impl std::fmt::Display for BvhBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLost => f.write_str("wgpu device was lost during the BVH build"),
            Self::BufferMapFailed => f.write_str("staging buffer map_async failed"),
            Self::OutOfMemory => f.write_str("GPU buffer allocation failed (out of memory)"),
        }
    }
}

impl std::error::Error for BvhBuildError {}

/// Shared state between the orchestrator's `map_async` callbacks and
/// [`BvhGpuBuild::poll`]. Atomic loads/stores on the booleans avoid
/// any locking on the hot poll path.
#[derive(Default)]
struct MapState {
    nodes_done: AtomicBool,
    indices_done: AtomicBool,
    nodes_err: AtomicBool,
    indices_err: AtomicBool,
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
}

/// Lightweight view of a completed (or in-flight + fenced) GPU BVH for
/// downstream traversal kernels. PR-4 raymarch culling + PR-5
/// broadphase consume this without ever going through CPU readback.
///
/// `nodes_buffer` is a borrow of the `BvhGpuBuild`'s refcounted clone
/// of the builder's nodes buffer — it stays valid for the lifetime of
/// the [`BvhGpuBuild`] (or longer; the underlying GPU buffer is shared
/// with the builder's reusable storage).
pub struct GpuBvhHandle<'a> {
    pub nodes_buffer: &'a wgpu::Buffer,
    pub n: u32,
}

impl<T: Copy> BvhGpuBuild<T> {
    /// Non-blocking check. Returns:
    ///
    /// - `None` while either staging buffer is still in flight.
    /// - `Some(Ok(Bvh<T>))` once both buffers are mapped — the result
    ///   is byte-identical to `Bvh::build(items)` on the same input.
    /// - `Some(Err(_))` if the device was lost or a buffer map failed.
    ///
    /// Caller is expected to call `device.poll(PollType::Poll)` once
    /// per frame so wgpu invokes the `map_async` callbacks; this
    /// function only reads the resulting atomic flags.
    pub fn poll(&mut self, _device: &wgpu::Device) -> Option<Result<Bvh<T>, BvhBuildError>> {
        if self.consumed {
            return None;
        }
        if self.submission_index.is_none() {
            // n == 0 path — empty BVH resolves immediately.
            self.consumed = true;
            return Some(Ok(Bvh::empty()));
        }
        if self.map_state.nodes_err.load(Ordering::Acquire)
            || self.map_state.indices_err.load(Ordering::Acquire)
        {
            self.consumed = true;
            return Some(Err(BvhBuildError::BufferMapFailed));
        }
        if !self.map_state.nodes_done.load(Ordering::Acquire)
            || !self.map_state.indices_done.load(Ordering::Acquire)
        {
            return None;
        }

        // Both staging buffers are mapped — assemble the Bvh<T>.
        self.consumed = true;
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
        Some(Ok(Bvh { nodes, leaves }))
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
    pub fn block_on(mut self, device: &wgpu::Device) -> Result<Bvh<T>, BvhBuildError> {
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

    let submission_index = queue.submit(std::iter::once(encoder.finish()));

    // Arm map_async on both staging buffers. Callbacks fire from
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

    BvhGpuBuild {
        n,
        submission_index: Some(submission_index),
        nodes_staging,
        indices_staging,
        nodes_buffer: builder.lbvh_buffers.nodes_buffer.clone(),
        map_state,
        payloads,
        consumed: false,
    }
}

/// Construct a placeholder `BvhGpuBuild` for the `n == 0` case. No GPU
/// dispatches and no submission — `submission_index = None` makes
/// [`BvhGpuBuild::poll`] return `Some(Ok(Bvh::empty()))` immediately.
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
    }
}

#[cfg(test)]
mod smoke {
    //! End-to-end smoke test for `Bvh::build_gpu`. The 6-size CPU/GPU
    //! consistency suite (N = 1, 2, 8, 100, 1024, 65 000) lives in the
    //! sibling commit (subtask 4d).

    use super::*;
    use crate::gpu::builder::test_device;
    use glam::Vec3;

    fn aabb_at(centre: Vec3, half: f32) -> Aabb {
        Aabb::from_centre(centre, Vec3::splat(half))
    }

    #[test]
    fn build_gpu_matches_cpu_n_8_smoke() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("ome_bvh::gpu::build: no GPU adapter — skipping");
            return;
        };
        let mut builder = BvhGpuBuilder::new(&device, &queue, None);

        let items: Vec<(u32, Aabb)> = (0..8u32)
            .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
            .collect();

        let cpu = Bvh::build(items.clone());
        let build = Bvh::<u32>::build_gpu(&mut builder, &device, &queue, items);
        let gpu = build.block_on(&device).expect("GPU build failed");

        assert_eq!(gpu.nodes.len(), cpu.nodes.len(), "node count");
        for (i, (g, c)) in gpu.nodes.iter().zip(cpu.nodes.iter()).enumerate() {
            assert_eq!(g, c, "node[{i}] diverges:\n  gpu: {g:?}\n  cpu: {c:?}");
        }
        assert_eq!(gpu.leaves, cpu.leaves, "leaves payload mismatch");
    }
}
