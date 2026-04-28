//! [`SharedBvhState`] — multi-consumer GPU BVH lifecycle.
//!
//! Owns a single [`BvhGpuBuilder`] and a double-buffered pair of
//! [`OutputSlot`]s (`nodes + sorted_indices + leaf_aabbs`). Every BVH
//! consumer (raymarch, physics broadphase, frustum culling) binds the
//! same three buffers from the currently-active slot — that is the
//! "shared" of the name (#115 PR-5 AC 116).
//!
//! Side payloads (e.g. the raymarch's per-primitive smoothness) are
//! NOT owned by this struct. Consumers that need them maintain their
//! own parallel double-buffers and mirror the swap by listening to
//! [`SwapInfo::target_slot`] from [`Self::poll_swap`].
//!
//! # Hashing contract
//!
//! [`Self::kick`] takes the scene hash from the caller rather than
//! computing it. This lets each consumer fold its side-payload bytes
//! into the hash before kicking — a smoothness-only change in
//! raymarch must still trigger a rebuild even though the items +
//! leaves are byte-identical.

use crate::leaf::LeafAabb;
use crate::{Aabb, Bvh, BvhBuildError, BvhGpuBuild, BvhGpuBuilder, BvhNode};

/// Initial capacity (in leaves) for the per-slot stable buffers.
const INITIAL_SLOT_CAPACITY: u64 = 256;

/// Per-slot stable buffer set. The renderer / physics / frustum-cull
/// consumers bind these directly when the slot is `current`. Capacity
/// grows on demand via `next_power_of_two`; buffers never shrink.
struct OutputSlot {
    /// Flat tree of [`BvhNode`]s. Sized for `2N` to keep capacity
    /// arithmetic simple (real fill is `2N - 1`).
    nodes_buffer: wgpu::Buffer,
    /// `sorted_indices[k]` = original payload index at sorted position
    /// `k`. Length `N`.
    sorted_indices_buffer: wgpu::Buffer,
    /// Per-primitive multi-consumer metadata (AABB + flags + entity_id).
    /// Length `N`.
    leaf_aabbs_buffer: wgpu::Buffer,
    capacity: u64,
    /// Number of valid leaves (= primitives) in this slot. `0` until
    /// the first build resolves into it.
    n: u32,
}

impl OutputSlot {
    fn new(device: &wgpu::Device, capacity: u64) -> Self {
        Self {
            nodes_buffer: make_nodes_buffer(device, capacity),
            sorted_indices_buffer: make_indices_buffer(device, capacity),
            leaf_aabbs_buffer: make_leaf_aabbs_buffer(device, capacity),
            capacity,
            n: 0,
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SLOT_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.sorted_indices_buffer = make_indices_buffer(device, new_cap);
        self.leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::shared::nodes"),
        size: 2 * capacity * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_indices_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::shared::sorted_indices"),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_leaf_aabbs_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::shared::leaf_aabbs"),
        size: capacity * std::mem::size_of::<LeafAabb>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// In-flight build awaiting GPU completion.
struct PendingBuild {
    build: BvhGpuBuild<u32>,
    /// Slot index (0 or 1) the result will land in.
    target_slot: u8,
    /// Per-leaf metadata captured at kick time. Uploaded to the target
    /// slot's `leaf_aabbs_buffer` at swap. Held on the CPU because the
    /// LBVH builder doesn't see this metadata.
    leaf_aabbs: Vec<LeafAabb>,
    /// Number of leaves submitted to the build.
    n: u32,
}

/// Information published when [`SharedBvhState::poll_swap`] resolves a
/// pending build. Side-payload consumers mirror their double-buffer
/// swap onto [`Self::target_slot`] and copy `n` items into it.
#[derive(Clone, Copy, Debug)]
pub struct SwapInfo {
    /// The slot index (0 or 1) that just became `current`. Side-
    /// payload consumers should upload their parallel data to this
    /// slot.
    pub target_slot: u8,
    /// Number of leaves in the resolved build.
    pub n: u32,
}

/// Multi-consumer double-buffered GPU BVH state. Held as a single
/// resource shared by every BVH consumer in the engine.
pub struct SharedBvhState {
    builder: BvhGpuBuilder,
    slot_a: OutputSlot,
    slot_b: OutputSlot,
    /// `0` → consumers read `slot_a`, build target is `slot_b`.
    current_slot: u8,
    pending: Option<PendingBuild>,
    /// Hash of the last successfully kicked scene state. Compared by
    /// the caller-supplied hash on the next kick.
    dirty_hash: Option<u64>,
}

impl SharedBvhState {
    /// Build the GPU compute infrastructure + initial empty slots.
    /// `pipeline_cache` is forwarded to the LBVH builder — pass the
    /// engine's shared `wgpu::PipelineCache` to amortise compile time.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let builder = BvhGpuBuilder::new(device, queue, pipeline_cache);
        Self {
            builder,
            slot_a: OutputSlot::new(device, INITIAL_SLOT_CAPACITY),
            slot_b: OutputSlot::new(device, INITIAL_SLOT_CAPACITY),
            current_slot: 0,
            pending: None,
            dirty_hash: None,
        }
    }

    /// Borrow the BVH-nodes buffer for the currently-active slot.
    pub fn current_nodes(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).nodes_buffer
    }

    /// Borrow the sorted-indices buffer for the currently-active slot.
    pub fn current_sorted_indices(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).sorted_indices_buffer
    }

    /// Borrow the leaf-AABB buffer for the currently-active slot.
    pub fn current_leaf_aabbs(&self) -> &wgpu::Buffer {
        &self.slot(self.current_slot).leaf_aabbs_buffer
    }

    /// Number of valid primitives in the currently-active slot. `0`
    /// before any build has resolved.
    pub fn current_n(&self) -> u32 {
        self.slot(self.current_slot).n
    }

    /// Index (0 or 1) of the slot consumers are currently reading.
    /// Side-payload double-buffers should bind their own slot at this
    /// index.
    pub fn current_slot_index(&self) -> u8 {
        self.current_slot
    }

    fn slot(&self, idx: u8) -> &OutputSlot {
        match idx {
            0 => &self.slot_a,
            _ => &self.slot_b,
        }
    }

    fn slot_mut(&mut self, idx: u8) -> &mut OutputSlot {
        match idx {
            0 => &mut self.slot_a,
            _ => &mut self.slot_b,
        }
    }

    /// Start a new GPU build if `scene_hash` differs from the last
    /// successfully kicked hash. Returns `true` when a new build was
    /// kicked, `false` when the kick was suppressed (either the hash
    /// matched or a previous build is still in flight).
    ///
    /// The caller supplies `scene_hash` so each consumer can fold its
    /// own side-payload bytes (raymarch smoothness, collider mass,
    /// etc.) into the hash. A pure rebuild of `items + leaf_aabbs`
    /// would miss those changes and silently render stale frames.
    pub fn kick(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        items: Vec<(u32, Aabb)>,
        leaf_aabbs: Vec<LeafAabb>,
        scene_hash: u64,
    ) -> bool {
        debug_assert_eq!(
            items.len(),
            leaf_aabbs.len(),
            "items and leaf_aabbs must align 1:1 — one entry per primitive",
        );
        if self.pending.is_some() {
            return false;
        }
        if Some(scene_hash) == self.dirty_hash {
            return false;
        }
        let n = items.len() as u32;
        let target_slot = self.current_slot ^ 1;

        let needed = (n as u64).max(1);
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let build = Bvh::<u32>::build_gpu(&mut self.builder, device, queue, items);
        self.pending = Some(PendingBuild {
            build,
            target_slot,
            leaf_aabbs,
            n,
        });
        self.dirty_hash = Some(scene_hash);
        true
    }

    /// Drive the in-flight build forward. Must be called once per
    /// frame. Returns `Some(SwapInfo)` on the frame the swap actually
    /// happens; consumers maintaining parallel double-buffers should
    /// upload their data to `info.target_slot`.
    ///
    /// - `None` — no pending build, or pending build still in flight.
    /// - `Some(Ok(info))` — pending build resolved; result copied into
    ///   the target slot; `current_slot` flipped.
    /// - `Some(Err(_))` — build failed; pending dropped without
    ///   touching the slots. Consumers keep using the previous slot.
    pub fn poll_swap(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Result<SwapInfo, BvhBuildError>> {
        device.poll(wgpu::PollType::Poll).ok()?;
        let pending = self.pending.as_mut()?;
        let outcome = pending.build.poll(device)?;
        let pending = self.pending.take().expect("just observed Some above");

        let bvh = match outcome {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };

        let n = pending.n;
        let target_slot = pending.target_slot;
        if n > 0 {
            let total_nodes = (2 * n - 1) as u64;
            let nodes_bytes = total_nodes * std::mem::size_of::<BvhNode>() as u64;
            let indices_bytes = (n as u64) * 4;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ome_bvh::shared::poll_swap_copy_encoder"),
            });
            encoder.copy_buffer_to_buffer(
                self.builder.nodes_buffer(),
                0,
                &self.slot(target_slot).nodes_buffer,
                0,
                nodes_bytes,
            );
            encoder.copy_buffer_to_buffer(
                self.builder.sorted_indices_buffer(),
                0,
                &self.slot(target_slot).sorted_indices_buffer,
                0,
                indices_bytes,
            );
            queue.submit(std::iter::once(encoder.finish()));
            queue.write_buffer(
                &self.slot(target_slot).leaf_aabbs_buffer,
                0,
                bytemuck::cast_slice(&pending.leaf_aabbs),
            );
        }
        self.slot_mut(target_slot).n = n;
        self.current_slot = target_slot;
        // Drop the build handle and the `bvh` Vec<BvhNode> the GPU
        // path doesn't need on the CPU side.
        drop(bvh);
        Some(Ok(SwapInfo { target_slot, n }))
    }
}
