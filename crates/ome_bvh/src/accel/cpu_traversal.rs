//! CPU-side mirror of the WGSL TLAS+BLAS descend.
//!
//! Walks the maintained CPU shadows of `tlas_nodes` + each chunk's
//! BLAS nodes + leaf AABBs (see [`super::ChunkSlot`]) without ever
//! paying for a GPU readback. The mirrors are populated synchronously
//! by `streaming::insert_chunk` / `refit_chunk` and `tlas::rebuild`,
//! so the CPU walk and the GPU shader observe **the same data** at
//! the same point in the pipeline.
//!
//! Today's only CPU consumer is `ome_physics::broadphase`: when
//! narrowphase eventually moves to GPU compute, the CPU broadphase
//! will be replaced by a GPU compute pass without changing the API
//! on the consumer side. The mirror layout described here is the
//! anchor that keeps the CPU and GPU traversals byte-for-byte
//! consistent during the migration.

use crate::aabb::Aabb;
use crate::accel::descriptor::ChunkDescriptor;
use crate::accel::state::OmeAccel;
use crate::accel::tlas;
use crate::leaf::LeafAabb;
use crate::node::{BVH_LEAF_FLAG, BVH_VALUE_MASK, BvhNode};

/// Traversal stack depth. Mirrors the WGSL `MAX_TLAS_STACK` /
/// `MAX_BLAS_STACK` (both 32) so any topology that fits the GPU also
/// fits the CPU walk.
const MAX_STACK: usize = 32;

/// `true` when `query` overlaps the AABB defined by `lo`/`hi`. AABB
/// overlap is `every-axis` `lo_query <= hi_node && hi_query >= lo_node`.
#[inline]
fn aabb_overlaps(query: Aabb, lo: [f32; 3], hi: [f32; 3]) -> bool {
    query.min.x <= hi[0]
        && query.max.x >= lo[0]
        && query.min.y <= hi[1]
        && query.max.y >= lo[1]
        && query.min.z <= hi[2]
        && query.max.z >= lo[2]
}

impl OmeAccel {
    /// Walk the pool with an AABB query and call `visit(prim_idx, leaf)`
    /// for **every** BLAS leaf whose AABB overlaps `query`. `prim_idx`
    /// is the absolute primitive index in `primitives_pool` —
    /// equivalent to the value `node.first_leaf()` carries on the GPU
    /// side.
    ///
    /// CPU mirror of the WGSL TLAS+BLAS descend in
    /// `raymarch_pool_eval.wgsl::eval_scene_bvh`, specialised to
    /// `aabb_overlaps` (any-overlap) instead of `aabb_contains`
    /// (point-in). Used by `ome_physics::broadphase`; usable by any
    /// future CPU narrowphase or editor inspector that needs the same
    /// topology the shader walks.
    ///
    /// Determinism: pushes left before right, pops right first — the
    /// same convention `eval_scene_bvh` uses, so the visit order is
    /// stable across CPU vs GPU traversals.
    ///
    /// Skips dead-flagged TLAS leaves (chunks evicted but not yet
    /// compacted out of the topology).
    pub fn for_each_overlapping_cpu<F>(&self, query: Aabb, mut visit: F)
    where
        F: FnMut(u32, &LeafAabb),
    {
        if self.cpu_tlas_nodes.is_empty() {
            return;
        }

        let mut tlas_stack: [u32; MAX_STACK] = [0; MAX_STACK];
        let mut sp: usize = 1;
        tlas_stack[0] = 0;

        while sp > 0 {
            sp -= 1;
            let node = &self.cpu_tlas_nodes[tlas_stack[sp] as usize];
            if !aabb_overlaps(query, node.aabb_min, node.aabb_max) {
                continue;
            }
            let payload = node.right_or_count;
            if (payload & BVH_LEAF_FLAG) != 0 {
                if tlas::is_dead(payload) {
                    continue;
                }
                let chunk_idx = tlas::decode_chunk_idx(payload) as usize;
                let slot = &self.slots[chunk_idx];
                if !slot.live {
                    continue;
                }
                descend_blas_cpu(query, &slot.descriptor, &slot.cpu_bvh_nodes,
                                 &slot.cpu_leaf_aabbs, &mut visit);
            } else {
                let left = node.left;
                let right = payload & BVH_VALUE_MASK;
                if sp + 2 <= MAX_STACK {
                    tlas_stack[sp] = left;
                    sp += 1;
                    tlas_stack[sp] = right;
                    sp += 1;
                }
            }
        }
    }
}

/// BLAS descend over `slot_nodes` (indexed from `descriptor.first_node`).
/// Calls `visit(prim_idx, leaf_aabb)` on every leaf whose AABB
/// overlaps `query`.
fn descend_blas_cpu<F>(
    query: Aabb,
    descriptor: &ChunkDescriptor,
    slot_nodes: &[BvhNode],
    slot_leaves: &[LeafAabb],
    visit: &mut F,
) where
    F: FnMut(u32, &LeafAabb),
{
    if slot_nodes.is_empty() {
        return;
    }
    let first_node = descriptor.first_node;
    let first_primitive = descriptor.first_primitive;

    let mut stack: [u32; MAX_STACK] = [0; MAX_STACK];
    let mut sp: usize = 1;
    stack[0] = first_node;

    while sp > 0 {
        sp -= 1;
        let node_idx_abs = stack[sp];
        // Slot stores `slot_nodes[0]` at pool index `first_node` —
        // walk in pool indices and translate to slot offsets.
        let local = (node_idx_abs - first_node) as usize;
        let node = &slot_nodes[local];
        if !aabb_overlaps(query, node.aabb_min, node.aabb_max) {
            continue;
        }
        let payload = node.right_or_count;
        if (payload & BVH_LEAF_FLAG) != 0 {
            // BLAS leaf — `node.left` is the absolute pool primitive
            // index (WGSL contract). The leaf metadata lives at the
            // matching offset in `slot_leaves`.
            let prim_idx = node.left;
            let leaf_local = (prim_idx - first_primitive) as usize;
            let leaf = &slot_leaves[leaf_local];
            visit(prim_idx, leaf);
        } else {
            let left = node.left;
            let right = payload & BVH_VALUE_MASK;
            if sp + 2 <= MAX_STACK {
                stack[sp] = left;
                sp += 1;
                stack[sp] = right;
                sp += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accel::{AccelCaps, ChunkInsert};
    use crate::leaf::IS_RAYMARCH;
    use glam::Vec3;

    fn skip_if_no_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ome_accel::cpu_traversal_tests"),
            required_features: wgpu::Features::empty(),
            // Default (full desktop) limits — `update_gpu` triggers
            // the GPU TLAS rebuild whose onesweep sort needs 6
            // storage buffers in one stage, exceeding downlevel.
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .ok()?;
        Some((device, queue))
    }

    fn leaf(centre_x: f32, entity_id: u32) -> LeafAabb {
        LeafAabb {
            aabb_min: [centre_x - 0.5, -0.5, -0.5],
            flags: IS_RAYMARCH,
            aabb_max: [centre_x + 0.5, 0.5, 0.5],
            entity_id,
        }
    }

    #[test]
    fn empty_pool_visits_nothing() {
        let Some((device, _queue)) = skip_if_no_device() else { return; };
        let accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let mut count = 0;
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::splat(-100.0), Vec3::splat(100.0)),
            |_, _| count += 1,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn single_chunk_overlap_visits_matching_leaves() {
        let Some((device, queue)) = skip_if_no_device() else { return; };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves: Vec<_> = (0..4).map(|i| leaf(i as f32 * 2.0, i)).collect();
        let primitives_bytes = vec![0u8; 16 * 4];
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: 0,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &primitives_bytes,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        accel.update_gpu(&queue, 0.0, 0.0);

        // Query a tight AABB around centre_x = 2 (leaf 1).
        let mut hit_ids = Vec::new();
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::new(1.6, -0.4, -0.4), Vec3::new(2.4, 0.4, 0.4)),
            |_, leaf| hit_ids.push(leaf.entity_id),
        );
        hit_ids.sort();
        assert_eq!(hit_ids, vec![1]);
    }

    #[test]
    fn multi_chunk_query_descends_into_each_overlapping_chunk() {
        let Some((device, queue)) = skip_if_no_device() else { return; };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        // Chunk A spans x ∈ [-1, 1]; chunk B spans x ∈ [9, 11].
        let leaves_a = vec![leaf(0.0, 100)];
        let leaves_b = vec![leaf(10.0, 200)];
        let prims_a = vec![0u8; 16];
        let prims_b = vec![0u8; 16];
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: 1,
                    leaf_aabbs: &leaves_a,
                    primitives_bytes: &prims_a,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: 2,
                    leaf_aabbs: &leaves_b,
                    primitives_bytes: &prims_b,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        accel.update_gpu(&queue, 0.0, 0.0);

        // Wide query — overlaps both chunks. Both leaves visited.
        let mut hit_ids = Vec::new();
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::new(-2.0, -2.0, -2.0), Vec3::new(12.0, 2.0, 2.0)),
            |_, leaf| hit_ids.push(leaf.entity_id),
        );
        hit_ids.sort();
        assert_eq!(hit_ids, vec![100, 200]);

        // Narrow query — overlaps only chunk B.
        let mut hit_ids = Vec::new();
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::new(9.6, -0.4, -0.4), Vec3::new(10.4, 0.4, 0.4)),
            |_, leaf| hit_ids.push(leaf.entity_id),
        );
        assert_eq!(hit_ids, vec![200]);
    }

    #[test]
    fn evicted_chunk_skipped_after_remove() {
        let Some((device, queue)) = skip_if_no_device() else { return; };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves = vec![leaf(0.0, 7)];
        let prims = vec![0u8; 16];
        accel
            .insert_chunk(
                &queue,
                ChunkInsert {
                    key: 1,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &prims,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        accel.update_gpu(&queue, 0.0, 0.0);

        let mut count = 0;
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::splat(-10.0), Vec3::splat(10.0)),
            |_, _| count += 1,
        );
        assert_eq!(count, 1);

        accel.remove_chunk(&queue, 1).unwrap();
        accel.update_gpu(&queue, 0.0, 0.0);

        let mut count = 0;
        accel.for_each_overlapping_cpu(
            Aabb::new(Vec3::splat(-10.0), Vec3::splat(10.0)),
            |_, _| count += 1,
        );
        assert_eq!(count, 0);
    }
}
