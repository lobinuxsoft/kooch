//! Per-frame uniforms upload + topology-preserving refit hook for
//! `OmeAccel`. Sits alongside `streaming/mod.rs` so the per-chunk
//! insert / remove / refit hot path stays under the workspace's 400
//! LoC monolith cap.

use bytemuck::cast_slice;
use std::mem::size_of;

use crate::accel::state::OmeAccel;
use crate::accel::tlas;
use crate::bvh::refit_slice_in_place;
use crate::leaf::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_INT, ROLE_RAYMARCH_MASK, ROLE_RAYMARCH_SUB};
use crate::node::BvhNode;

impl OmeAccel {
    /// Drive the TLAS rebuild + uniforms upload. Call once per frame
    /// before the raymarch dispatch — the streaming layer batches as
    /// many `insert_chunk` / `remove_chunk` / `refit_chunk` calls as
    /// it likes between two `update_gpu` calls; the rebuild collapses
    /// them into a single upload.
    ///
    /// Computes scene-wide `has_intersects` / `has_subs` flags by
    /// scanning every live chunk's leaf flags — `O(live_primitives)`
    /// per call. The shader uses the flags to skip the precision-
    /// lossy `smooth_intersection` / `smooth_subtraction` final-combine
    /// steps when the corresponding role is empty (radv `mix(a, b, t)`
    /// loses the smaller operand at the `±1e6` identities).
    pub fn update_gpu(&mut self, queue: &wgpu::Queue, k_int_global: f32, k_sub_global: f32) {
        if self.tlas_dirty_count > 0 {
            // TODO(PR-1 commit 9): hoist the encoder to the caller so
            // the TLAS rebuild co-batches with the renderer's per-frame
            // encoder instead of forcing a standalone submission.
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ome_accel::tlas_rebuild_encoder_TEMP"),
                });
            tlas::rebuild(self, &mut encoder, queue);
            queue.submit(std::iter::once(encoder.finish()));
            self.tlas_dirty_count = 0;
        }
        // Coalesce the BLAS free lists once per frame so post-eviction
        // re-inserts walk a sorted free list and keep `high_watermark`
        // tight (AC7's `used / high_watermark` invariant). `O(F log F)`
        // per pool — `F` is the disjoint range count, typically small
        // even under aggressive churn.
        self.free_node_ranges.coalesce();
        self.free_leaf_ranges.coalesce();
        self.free_primitive_ranges.coalesce();
        let mut has_intersects = 0u32;
        let mut has_subs = 0u32;
        for slot in &self.slots {
            if !slot.live {
                continue;
            }
            for la in &slot.cpu_leaf_aabbs {
                if la.flags & IS_RAYMARCH == 0 {
                    continue;
                }
                match la.flags & ROLE_RAYMARCH_MASK {
                    ROLE_RAYMARCH_INT => has_intersects = 1,
                    ROLE_RAYMARCH_SUB => has_subs = 1,
                    _ => {}
                }
            }
        }
        let uniforms = crate::accel::descriptor::TlasUniforms {
            k_int_global,
            k_sub_global,
            num_chunks: self.live_chunk_count(),
            has_intersects,
            has_subs,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        queue.write_buffer(
            &self.buffers.tlas_uniforms,
            0,
            bytemuck::bytes_of(&uniforms),
        );
    }

    /// Topology-preserving slice refit (no rebuild). Lives here as a
    /// follow-up optimisation hook for `refit_chunk` — kept exposed so
    /// downstream perf tests can target it directly.
    #[doc(hidden)]
    pub fn refit_chunk_slice_only(
        &mut self,
        queue: &wgpu::Queue,
        chunk_idx: u32,
        leaf_aabbs_perm: &[LeafAabb],
        nodes_dst: &mut [BvhNode],
    ) {
        let slot = &self.slots[chunk_idx as usize];
        let descriptor = slot.descriptor;
        let n = descriptor.leaf_count as usize;
        // The caller-owned `nodes_dst` is the existing pool slice
        // mirrored to CPU memory; refit in place.
        refit_slice_in_place(nodes_dst, n, leaf_aabbs_perm, &slot.sorted_indices);
        queue.write_buffer(
            &self.buffers.bvh_nodes_pool,
            descriptor.first_node as u64 * size_of::<BvhNode>() as u64,
            cast_slice(&nodes_dst[..if n == 1 { 1 } else { 2 * n - 1 }]),
        );
    }
}
