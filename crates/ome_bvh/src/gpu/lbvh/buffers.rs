use wgpu::util::DeviceExt;

use super::{INITIAL_LBVH_CAPACITY, LbvhConfig};
use crate::node::BvhNode;

/// Reusable GPU buffers for the LBVH build. Grow on demand, never
/// realloc'd per build (per the production-from-day-1 rule).
pub struct LbvhBuffers {
    /// Output flat tree, `2N-1` [`BvhNode`] entries.
    pub nodes_buffer: wgpu::Buffer,
    /// Parent index for every node (`2N-1` entries). Written by the
    /// internal pass; reserved for future GPU traversal kernels (PR-4
    /// raymarch culling, PR-5 collision broadphase). Not read by the
    /// AABB propagation pass.
    pub parents_buffer: wgpu::Buffer,
    /// `done[node]` flag (`2N-1` entries) — leaves are finalised by
    /// pass 1, internals by pass 3 once their two children are done.
    pub done_buffer: wgpu::Buffer,
    /// Capacity in *leaves count* (N).
    pub capacity: u64,
    /// Uniform with `n` for every dispatch — written via
    /// `queue.write_buffer` at the start of each build.
    pub config_buffer: wgpu::Buffer,
}

impl LbvhBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let nodes_buffer = make_nodes_buffer(device, INITIAL_LBVH_CAPACITY);
        let parents_buffer = make_aux_u32_buffer(device, "lbvh_parents", INITIAL_LBVH_CAPACITY);
        let done_buffer = make_aux_u32_buffer(device, "lbvh_done", INITIAL_LBVH_CAPACITY);
        let config_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ome_bvh::lbvh_config"),
            contents: bytemuck::bytes_of(&LbvhConfig::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            nodes_buffer,
            parents_buffer,
            done_buffer,
            capacity: INITIAL_LBVH_CAPACITY,
            config_buffer,
        }
    }

    /// Grow the LBVH buffers if `n_leaves` exceeds the current
    /// capacity. New capacity is `next_power_of_two(n_leaves)`.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, n_leaves: u64) {
        if n_leaves <= self.capacity {
            return;
        }
        let new_cap = n_leaves.next_power_of_two().max(INITIAL_LBVH_CAPACITY);
        self.nodes_buffer = make_nodes_buffer(device, new_cap);
        self.parents_buffer = make_aux_u32_buffer(device, "lbvh_parents", new_cap);
        self.done_buffer = make_aux_u32_buffer(device, "lbvh_done", new_cap);
        self.capacity = new_cap;
    }
}

fn make_nodes_buffer(device: &wgpu::Device, n: u64) -> wgpu::Buffer {
    // 2N-1 nodes — round to 2N for capacity arithmetic simplicity.
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::lbvh_nodes"),
        size: 2 * n * std::mem::size_of::<BvhNode>() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_aux_u32_buffer(device: &wgpu::Device, suffix: &str, n: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("ome_bvh::{suffix}")),
        size: 2 * n * 4,
        // `COPY_SRC` is needed by the `cfg(debug_assertions)` AABB
        // convergence invariant check in `gpu::build` (it copies the
        // `done_buffer` to a MAP_READ staging buffer). The flag is
        // free in release — wgpu only validates against pipeline
        // usage at command submission time.
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}
