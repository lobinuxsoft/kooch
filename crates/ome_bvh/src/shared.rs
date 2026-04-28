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
use crate::{Aabb, Bvh, BvhBuildError, BvhGpuBuild, BvhGpuBuilder, BvhGpuRefit, BvhNode};

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

/// Discriminator between an in-flight full build and an in-flight
/// refit. Both operate on the builder's scratch buffers and resolve
/// at the same point in the lifecycle ([`SharedBvhState::poll_swap`]),
/// so the orchestrator carries them through one common state slot.
enum PendingKind {
    Build(BvhGpuBuild<u32>),
    Refit(BvhGpuRefit),
}

impl PendingKind {
    fn poll(&mut self, device: &wgpu::Device) -> Option<Result<(), BvhBuildError>> {
        match self {
            // BvhGpuBuild::poll returns the full Bvh<T>; we drop it
            // (the orchestrator path stays GPU-resident).
            Self::Build(op) => op.poll(device).map(|r| r.map(|_| ())),
            Self::Refit(op) => op.poll(device),
        }
    }
}

/// In-flight build or refit awaiting GPU completion.
struct Pending {
    op: PendingKind,
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
    pending: Option<Pending>,
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
        self.pending = Some(Pending {
            op: PendingKind::Build(build),
            target_slot,
            leaf_aabbs,
            n,
        });
        self.dirty_hash = Some(scene_hash);
        true
    }

    /// Refit fast path: rewrite leaves with new AABBs and propagate
    /// internals over the existing topology, skipping morton + sort
    /// + Karras' internal-node pass. Returns `true` when a refit was
    /// kicked, `false` when the kick was suppressed.
    ///
    /// Suppressed when:
    /// - A previous build / refit is still in flight.
    /// - `scene_hash` matches the last successfully kicked hash.
    /// - The current slot's `n` does not match `items.len()` (refit
    ///   requires the same cardinality + ordering as the previous
    ///   build; otherwise the caller should fall back to [`Self::kick`]).
    /// - There is no previous build in the builder's scratch (i.e.
    ///   `current_n() == 0`); a refit has nothing to start from.
    ///
    /// **Caller invariants** for a successful refit (silent corruption
    /// otherwise):
    /// - `items[i].0` is at the same array position as in the last
    ///   build. Only the AABBs are allowed to change.
    /// - The previous build's outputs still live in the builder's
    ///   scratch (no intermediate failed [`Self::kick`] has clobbered
    ///   them; failed kicks discard `pending` cleanly so this is true
    ///   in practice).
    pub fn kick_refit(
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
        if n == 0 || self.slot(self.current_slot).n != n {
            // Refit invariant: cardinality must match the previous
            // build. Caller must use kick() instead.
            return false;
        }
        let target_slot = self.current_slot ^ 1;
        let needed = n as u64;
        self.slot_mut(target_slot).ensure_capacity(device, needed);

        let refit = crate::gpu::refit_gpu::<u32>(&mut self.builder, device, queue, items);
        self.pending = Some(Pending {
            op: PendingKind::Refit(refit),
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
        let outcome = pending.op.poll(device)?;
        let pending = self.pending.take().expect("just observed Some above");

        if let Err(e) = outcome {
            return Some(Err(e));
        }

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
        Some(Ok(SwapInfo { target_slot, n }))
    }
}

/// Cheap CPU heuristic for the orchestrator: decide between rebuild
/// (full [`SharedBvhState::kick`]) and refit (fast
/// [`SharedBvhState::kick_refit`]) based on how much each AABB has
/// moved relative to its size.
///
/// Returns `true` (i.e. refit is fine) when **fewer than
/// `change_threshold_pct`** of the AABBs have moved their centre by
/// **more than `move_threshold_ratio`** of their largest extent. Any
/// stretch / shrink that keeps the centre in place is treated as a
/// non-move — this is the cheap proxy; a tighter check would compare
/// volumes too, but the rebuild fallback is safe and fast enough that
/// the simple metric earns its keep.
///
/// Returns `false` (force rebuild) when:
/// - The lengths differ (cardinality changed → refit not viable).
/// - The previous slice is empty (first frame).
/// - The configured percentage of AABBs moved too far.
///
/// Suggested defaults from the PR-5 plan: `move_threshold_ratio =
/// 0.25`, `change_threshold_pct = 10.0`. These are conservative
/// (favour rebuild) — the S7 bench surfaces tighter values once the
/// real workload tells us what "moderate movement" means in practice.
pub fn should_refit(
    prev: &[Aabb],
    curr: &[Aabb],
    move_threshold_ratio: f32,
    change_threshold_pct: f32,
) -> bool {
    if prev.len() != curr.len() || prev.is_empty() {
        return false;
    }
    let mut moved = 0usize;
    for (p, c) in prev.iter().zip(curr.iter()) {
        let extent = p.max - p.min;
        let max_dim = extent.x.max(extent.y).max(extent.z).max(1e-6);
        let centre_delta = (c.center() - p.center()).length();
        if centre_delta > max_dim * move_threshold_ratio {
            moved += 1;
        }
    }
    let pct_moved = moved as f32 / prev.len() as f32 * 100.0;
    pct_moved < change_threshold_pct
}
