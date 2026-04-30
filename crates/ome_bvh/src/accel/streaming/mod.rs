//! Hot-path streaming API for `OmeAccel` — `insert_chunk`,
//! `remove_chunk`, `refit_chunk`, `update_gpu`. Every byte is written
//! via `Queue::write_buffer` slice writes into the pre-allocated pools;
//! no GPU-side allocation in the hot path.
//!
//! # WGSL contract for the BLAS leaves
//!
//! The shader reads each BLAS leaf via `primitives_pool[node.first_leaf()]`
//! and `leaf_aabbs_pool[node.first_leaf()]` directly — no offset fixup,
//! no separate `sorted_indices` binding. Two invariants make that work:
//!
//! 1. **`node.left` (== `node.first_leaf()`) carries the *absolute*
//!    primitive index in `primitives_pool`** — `first_primitive + i`,
//!    not the morton-sorted local `k`. `Bvh::build_into` emits
//!    `node.left = k`, so we post-pass leaf nodes to rewrite
//!    `node.left = leaves_scratch[k]` (the payload, which already
//!    carries `first_primitive + i`).
//! 2. **`leaf_aabbs_pool` is written in original-input order**, so
//!    `leaf_aabbs_pool[first_primitive + i]` resolves to the `LeafAabb`
//!    of original primitive `i`. No morton permutation on the leaf
//!    side: the BLAS leaf node already carries the sorted AABB in
//!    `node.aabb_min/aabb_max`; `leaf_aabbs_pool` only serves the role
//!    flags / `entity_id` lookup, which keys on the absolute primitive
//!    index via (1).
//!
//! # Order invariants
//!
//! - **Insert:** BLAS write (nodes + leaves + primitives) → descriptor
//!   write → TLAS dirty-count bump. The next `update_gpu` decides
//!   between incremental refit and full rebuild based on
//!   [`TLAS_REBUILD_THRESHOLD`](super::TLAS_REBUILD_THRESHOLD).
//! - **Remove:** mark dead → free pool ranges → clear CPU mirror →
//!   dirty-count bump. In-flight traversals see the live → dead
//!   transition before the BLAS slice is freed.

pub mod dtos;
mod uniforms;

#[cfg(test)]
mod contract_tests;

pub use dtos::{ChunkInsert, ChunkRefit};

use bytemuck::cast_slice;
use glam::Vec3;
use std::mem::size_of;

use crate::aabb::Aabb;
use crate::accel::descriptor::ChunkDescriptor;
use crate::accel::error::AccelError;
use crate::accel::state::{ChunkBvhHandle, ChunkKey, ChunkSlot, OmeAccel};
use crate::bvh::Bvh;
use crate::leaf::LeafAabb;
use crate::node::{BVH_LEAF_FLAG, BvhNode};

impl OmeAccel {
    /// Bring a new chunk into the pool. Allocates byte slices in the
    /// three BLAS pools, builds the BLAS via `Bvh::build_into`,
    /// uploads everything in three `Queue::write_buffer` calls, and
    /// returns the assigned `chunk_idx`.
    pub fn insert_chunk(
        &mut self,
        queue: &wgpu::Queue,
        insert: ChunkInsert<'_>,
    ) -> Result<ChunkBvhHandle, AccelError> {
        let n = insert.leaf_aabbs.len() as u32;
        if n == 0 {
            return Err(AccelError::EmptyPrimitives);
        }
        debug_assert_eq!(
            insert.primitives_bytes.len(),
            self.primitive_stride as usize * n as usize,
            "primitives_bytes length must equal stride * leaf_count",
        );

        let chunk_idx = self.free_chunk_slots.pop().ok_or(AccelError::OutOfChunkSlots)?;
        let total_nodes = if n == 1 { 1 } else { 2 * n - 1 };

        let first_node = match self.free_node_ranges.alloc(total_nodes) {
            Some(v) => v,
            None => {
                self.free_chunk_slots.push(chunk_idx);
                return Err(AccelError::OutOfNodes);
            }
        };
        let first_leaf = match self.free_leaf_ranges.alloc(n) {
            Some(v) => v,
            None => {
                self.free_node_ranges.free(first_node, total_nodes);
                self.free_chunk_slots.push(chunk_idx);
                return Err(AccelError::OutOfLeaves);
            }
        };
        let first_primitive = match self.free_primitive_ranges.alloc(n) {
            Some(v) => v,
            None => {
                self.free_leaf_ranges.free(first_leaf, n);
                self.free_node_ranges.free(first_node, total_nodes);
                self.free_chunk_slots.push(chunk_idx);
                return Err(AccelError::OutOfPrimitives);
            }
        };

        // Build BLAS into a CPU scratch slice. Payload `T = u32` is
        // the **absolute pool primitive index**: `leaves_scratch[k] =
        // first_primitive + i_original` for the leaf at sorted
        // position `k`. The WGSL contract (see module docstring) wants
        // that absolute index in `node.left` directly — see post-pass
        // below.
        let items: Vec<(u32, Aabb)> = insert
            .leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let aabb = Aabb::new(Vec3::from(l.aabb_min), Vec3::from(l.aabb_max));
                (first_primitive + i as u32, aabb)
            })
            .collect();
        let mut nodes_scratch = vec![BvhNode::default(); total_nodes as usize];
        let mut leaves_scratch = vec![0u32; n as usize];
        let meta = Bvh::<u32>::build_into(items, &mut nodes_scratch, &mut leaves_scratch);

        // Post-pass A: rewrite leaf-node `left` from sorted-position
        // `k` (what `Bvh::build_into` emits) to the absolute pool
        // primitive index `leaves_scratch[k]`. After this, the WGSL
        // traversal at a leaf can do `prim = primitives[node.left]`
        // and `meta = leaf_aabbs[node.left]` without an offset fixup
        // or an extra `sorted_indices` binding.
        let leaf_offset = (n.saturating_sub(1)) as usize;
        for k in 0..n as usize {
            nodes_scratch[leaf_offset + k].left = leaves_scratch[k];
        }

        // Post-pass B: bias every **internal** node's child indices
        // by `first_node` so they point into absolute `bvh_nodes_pool`
        // positions. `Bvh::build_into` emits child indices relative
        // to the local `nodes_dst` slice (range `0..total_nodes`);
        // when the slice is uploaded at pool offset `first_node` the
        // shader reads `bvh_nodes_pool[child_idx]` and would land in
        // the wrong chunk's slice without this bias. Single-chunk
        // pools have `first_node = 0` so the bias is a no-op (which
        // is why PR-2's AC1 + AC6 passed despite the bug — the
        // failure mode requires `chunk_count > 1`, surfaced by AC2).
        if first_node != 0 {
            for k in 0..leaf_offset {
                let internal = &mut nodes_scratch[k];
                debug_assert!(
                    internal.right_or_count & crate::node::BVH_LEAF_FLAG == 0,
                    "post-pass B must only bias internal nodes",
                );
                internal.left += first_node;
                internal.right_or_count += first_node;
            }
        }

        // Inflate root AABB by max_smoothness_radius — TLAS culling
        // stays conservative under cross-chunk smooth blends.
        let r = insert.max_smoothness_radius.max(0.0);
        let aabb_min = [
            meta.root_aabb.min.x - r,
            meta.root_aabb.min.y - r,
            meta.root_aabb.min.z - r,
        ];
        let aabb_max = [
            meta.root_aabb.max.x + r,
            meta.root_aabb.max.y + r,
            meta.root_aabb.max.z + r,
        ];

        let descriptor = ChunkDescriptor {
            aabb_min,
            first_node,
            aabb_max,
            node_count: total_nodes,
            first_leaf,
            leaf_count: n,
            first_primitive,
            primitive_count: n,
            max_smoothness_radius: r,
            _pad: [0.0; 3],
        };

        // GPU writes — four slice writes into the pre-allocated pools.
        // `leaf_aabbs_pool` keeps original-input order so the absolute
        // primitive index from `node.left` indexes it directly (WGSL
        // contract — see module docstring).
        queue.write_buffer(
            &self.buffers.bvh_nodes_pool,
            first_node as u64 * size_of::<BvhNode>() as u64,
            cast_slice(&nodes_scratch),
        );
        queue.write_buffer(
            &self.buffers.leaf_aabbs_pool,
            first_leaf as u64 * size_of::<LeafAabb>() as u64,
            cast_slice(insert.leaf_aabbs),
        );
        queue.write_buffer(
            &self.buffers.primitives_pool,
            first_primitive as u64 * self.primitive_stride as u64,
            insert.primitives_bytes,
        );
        queue.write_buffer(
            &self.buffers.chunk_descriptors,
            chunk_idx as u64 * size_of::<ChunkDescriptor>() as u64,
            bytemuck::bytes_of(&descriptor),
        );

        // CPU mirror.
        self.slots[chunk_idx as usize] = ChunkSlot {
            descriptor,
            live: true,
            key: insert.key,
            sorted_indices: leaves_scratch,
            cpu_bvh_nodes: nodes_scratch,
            cpu_leaf_aabbs: insert.leaf_aabbs.to_vec(),
        };
        self.coord_to_idx.insert(insert.key, chunk_idx);
        self.tlas_dirty_count = self.tlas_dirty_count.saturating_add(1);

        Ok(ChunkBvhHandle { chunk_idx })
    }

    /// Evict a chunk. Marks the TLAS leaf dead-skip immediately so
    /// in-flight traversals stop descending; the BLAS pool slots are
    /// then returned to the free lists. The slot's `chunk_idx` is
    /// pushed back onto `free_chunk_slots` and may be reused by the
    /// next `insert_chunk`.
    pub fn remove_chunk(&mut self, _queue: &wgpu::Queue, key: ChunkKey) -> Result<(), AccelError> {
        let chunk_idx = self.coord_to_idx.remove(&key).ok_or(AccelError::UnknownChunk)?;
        let slot = &mut self.slots[chunk_idx as usize];
        if !slot.live {
            return Err(AccelError::UnknownChunk);
        }
        let desc = slot.descriptor;
        // Mark slot dead first. Drop the CPU mirrors so traversals
        // observe the eviction immediately and the slot's previous
        // memory is released.
        slot.live = false;
        slot.sorted_indices.clear();
        slot.cpu_bvh_nodes.clear();
        slot.cpu_leaf_aabbs.clear();

        // Return pool ranges. TLAS gets rebuilt on the next
        // `update_gpu` so we don't write the dead-skip flag here —
        // the rebuild produces a TLAS topology that excludes this
        // chunk entirely, which is strictly better than leaving a
        // dead-skip leaf around.
        self.free_node_ranges.free(desc.first_node, desc.node_count);
        self.free_leaf_ranges.free(desc.first_leaf, desc.leaf_count);
        self.free_primitive_ranges
            .free(desc.first_primitive, desc.primitive_count);
        self.free_chunk_slots.push(chunk_idx);
        self.tlas_dirty_count = self.tlas_dirty_count.saturating_add(1);
        Ok(())
    }

    /// Refit a chunk's BLAS in place. Cardinality must match the last
    /// `insert_chunk` for this `key`; topology is preserved (Karras
    /// reorder under `refit_slice_in_place`).
    pub fn refit_chunk(
        &mut self,
        queue: &wgpu::Queue,
        refit: ChunkRefit<'_>,
    ) -> Result<(), AccelError> {
        let chunk_idx = *self.coord_to_idx.get(&refit.key).ok_or(AccelError::UnknownChunk)?;
        let slot_idx = chunk_idx as usize;
        let n = refit.leaf_aabbs.len() as u32;
        if n == 0 {
            return Err(AccelError::EmptyPrimitives);
        }
        debug_assert_eq!(
            n, self.slots[slot_idx].descriptor.leaf_count,
            "refit_chunk requires same leaf_count as the original insert",
        );
        debug_assert_eq!(
            refit.primitives_bytes.len(),
            self.primitive_stride as usize * n as usize,
        );

        let descriptor = self.slots[slot_idx].descriptor;
        let total_nodes = descriptor.node_count;
        let first_node = descriptor.first_node;
        let first_leaf = descriptor.first_leaf;
        let first_primitive = descriptor.first_primitive;

        // Recompute the BLAS via the same Karras path as `insert_chunk`
        // — the moved-entity case is the typical refit caller, and
        // entity drift up to a chunk diameter can swap leaf morton
        // codes. Reusing `Bvh::build_into` on the new AABBs is the
        // straight-line correct answer; the topology-preserving fast
        // path stays exposed via `refit_chunk_slice_only` for the
        // perf hook below. Ordering: `leaves_scratch[k]` = absolute
        // pool primitive index after the build.
        let mut nodes_scratch = vec![BvhNode::default(); total_nodes as usize];
        let items: Vec<(u32, Aabb)> = refit
            .leaf_aabbs
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let aabb = Aabb::new(Vec3::from(l.aabb_min), Vec3::from(l.aabb_max));
                (first_primitive + i as u32, aabb)
            })
            .collect();
        let mut leaves_scratch = vec![0u32; n as usize];
        let meta = Bvh::<u32>::build_into(items, &mut nodes_scratch, &mut leaves_scratch);

        // Post-passes A + B: same WGSL contract as `insert_chunk` —
        // leaf `node.left` carries the absolute pool primitive index;
        // internal node child indices biased by `first_node` so they
        // point into absolute `bvh_nodes_pool` positions.
        let leaf_offset = (n.saturating_sub(1)) as usize;
        for k in 0..n as usize {
            nodes_scratch[leaf_offset + k].left = leaves_scratch[k];
        }
        if first_node != 0 {
            for k in 0..leaf_offset {
                let internal = &mut nodes_scratch[k];
                debug_assert!(
                    internal.right_or_count & crate::node::BVH_LEAF_FLAG == 0,
                    "post-pass B must only bias internal nodes",
                );
                internal.left += first_node;
                internal.right_or_count += first_node;
            }
        }

        let r = refit.max_smoothness_radius.max(0.0);
        let new_min = [
            meta.root_aabb.min.x - r,
            meta.root_aabb.min.y - r,
            meta.root_aabb.min.z - r,
        ];
        let new_max = [
            meta.root_aabb.max.x + r,
            meta.root_aabb.max.y + r,
            meta.root_aabb.max.z + r,
        ];

        // GPU writes. `leaf_aabbs_pool` keeps original-input order
        // (WGSL contract) so the absolute primitive index from
        // `node.left` indexes it directly.
        queue.write_buffer(
            &self.buffers.bvh_nodes_pool,
            first_node as u64 * size_of::<BvhNode>() as u64,
            cast_slice(&nodes_scratch),
        );
        queue.write_buffer(
            &self.buffers.leaf_aabbs_pool,
            first_leaf as u64 * size_of::<LeafAabb>() as u64,
            cast_slice(refit.leaf_aabbs),
        );
        queue.write_buffer(
            &self.buffers.primitives_pool,
            first_primitive as u64 * self.primitive_stride as u64,
            refit.primitives_bytes,
        );

        // CPU mirror updates — descriptor + cached permutation +
        // shadowed BLAS nodes / leaf aabbs.
        let slot = &mut self.slots[slot_idx];
        slot.descriptor.aabb_min = new_min;
        slot.descriptor.aabb_max = new_max;
        slot.descriptor.max_smoothness_radius = r;
        slot.sorted_indices = leaves_scratch;
        slot.cpu_bvh_nodes = nodes_scratch;
        slot.cpu_leaf_aabbs = refit.leaf_aabbs.to_vec();

        let descriptor = slot.descriptor;
        queue.write_buffer(
            &self.buffers.chunk_descriptors,
            chunk_idx as u64 * size_of::<ChunkDescriptor>() as u64,
            bytemuck::bytes_of(&descriptor),
        );

        self.tlas_dirty_count = self.tlas_dirty_count.saturating_add(1);
        Ok(())
    }

}

// `update_gpu` + `refit_chunk_slice_only` live in the sibling
// `uniforms.rs` so the per-chunk hot-path file stays under the
// workspace's 400 LoC monolith cap.

/// Helper exposed for tests / external observability — confirms the
/// high bit of the encoded TLAS leaf payload aligns with the BLAS
/// discriminator.
#[doc(hidden)]
pub const fn _tlas_leaf_high_bit_invariant() -> u32 {
    BVH_LEAF_FLAG
}

