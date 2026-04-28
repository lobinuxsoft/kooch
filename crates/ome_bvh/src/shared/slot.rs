//! Per-slot stable buffer set for the double-buffered shared BVH.
//!
//! [`OutputSlot`] keeps the `nodes`, `sorted_indices`, and
//! `leaf_aabbs` buffers each consumer binds. Capacity grows on demand
//! via `next_power_of_two`; buffers never shrink — keeps reallocation
//! churn bounded across scene-size changes.

use crate::leaf::LeafAabb;
use crate::node::BvhNode;

/// Initial capacity (in leaves) for the per-slot stable buffers.
pub(super) const INITIAL_SLOT_CAPACITY: u64 = 256;

/// Per-slot stable buffer set. The renderer / physics / frustum-cull
/// consumers bind these directly when the slot is `current`. Capacity
/// grows on demand; buffers never shrink.
pub(super) struct OutputSlot {
    /// Flat tree of [`BvhNode`]s. Sized for `2N` to keep capacity
    /// arithmetic simple (real fill is `2N - 1`).
    pub(super) nodes_buffer: wgpu::Buffer,
    /// `sorted_indices[k]` = original payload index at sorted position
    /// `k`. Length `N`.
    pub(super) sorted_indices_buffer: wgpu::Buffer,
    /// Per-primitive multi-consumer metadata (AABB + flags + entity_id).
    /// Length `N`.
    pub(super) leaf_aabbs_buffer: wgpu::Buffer,
    pub(super) capacity: u64,
    /// Number of valid leaves (= primitives) in this slot. `0` until
    /// the first build resolves into it.
    pub(super) n: u32,
}

impl OutputSlot {
    pub(super) fn new(device: &wgpu::Device, capacity: u64) -> Self {
        Self {
            nodes_buffer: make_nodes_buffer(device, capacity),
            sorted_indices_buffer: make_indices_buffer(device, capacity),
            leaf_aabbs_buffer: make_leaf_aabbs_buffer(device, capacity),
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
