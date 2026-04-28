//! Per-slot stable buffer set for the double-buffered BVH state.
//!
//! `BvhState` keeps two `OutputSlot`s and rotates between them on every
//! resolved build. While slot A is bound to the renderer, the next
//! build's results are copied into slot B; on swap, the roles flip.
//! Capacity grows on demand via `next_power_of_two`; buffers never
//! shrink — keeps reallocation churn bounded across scene size changes.

use ome_bvh::BvhNode;

use crate::raymarch::instance::{LeafAabb, RaymarchPayload};

/// Initial capacity (in leaves) for the per-slot stable buffers. Grows
/// by `next_power_of_two` whenever a build exceeds it.
pub(super) const INITIAL_SLOT_CAPACITY: u64 = 256;

/// Per-slot stable buffer set. The renderer binds these directly when
/// the slot is `current`. Capacity grows on demand; buffers never shrink.
pub(super) struct OutputSlot {
    /// Flat tree of `BvhNode`s. Sized for `2N` to keep capacity
    /// arithmetic simple (real fill is `2N - 1`).
    pub(super) nodes_buffer: wgpu::Buffer,
    /// `sorted_indices[k]` = original payload index at sorted position
    /// `k`. Length `N`.
    pub(super) sorted_indices_buffer: wgpu::Buffer,
    /// Per-primitive multi-consumer metadata (AABB + flags + entity_id).
    /// Bound by every BVH consumer (raymarch, broadphase, frustum cull).
    /// Length `N`.
    pub(super) leaf_aabbs_buffer: wgpu::Buffer,
    /// Per-primitive raymarch-only metadata (smoothness). Bound only by
    /// the raymarch fragment shader; physics + frustum never read it.
    /// Length `N`.
    pub(super) raymarch_payloads_buffer: wgpu::Buffer,
    /// Capacity in leaves (matches `LbvhBuffers::capacity`'s convention).
    pub(super) capacity: u64,
    /// Number of valid leaves (= primitives) in this slot. `0` until the
    /// first build resolves into it.
    pub(super) n: u32,
}

impl OutputSlot {
    pub(super) fn new(device: &wgpu::Device, capacity: u64) -> Self {
        let nodes_buffer = make_nodes_buffer(device, capacity);
        let sorted_indices_buffer = make_indices_buffer(device, capacity);
        let leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, capacity);
        let raymarch_payloads_buffer = make_raymarch_payloads_buffer(device, capacity);
        Self {
            nodes_buffer,
            sorted_indices_buffer,
            leaf_aabbs_buffer,
            raymarch_payloads_buffer,
            capacity,
            n: 0,
        }
    }

    pub(super) fn ensure_capacity(&mut self, device: &wgpu::Device, n: u64) {
        if n <= self.capacity {
            return;
        }
        let new_cap = n.next_power_of_two().max(INITIAL_SLOT_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.sorted_indices_buffer = make_indices_buffer(device, new_cap);
        self.leaf_aabbs_buffer = make_leaf_aabbs_buffer(device, new_cap);
        self.raymarch_payloads_buffer = make_raymarch_payloads_buffer(device, new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::nodes"),
        size: 2 * capacity * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_indices_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::sorted_indices"),
        size: capacity * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_leaf_aabbs_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::leaf_aabbs"),
        size: capacity * std::mem::size_of::<LeafAabb>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_raymarch_payloads_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_bvh::slot::raymarch_payloads"),
        size: capacity * std::mem::size_of::<RaymarchPayload>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
