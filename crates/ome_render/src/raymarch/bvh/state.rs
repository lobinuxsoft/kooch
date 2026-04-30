//! [`BvhState`] — raymarch wrapper around `OmeAccel`.
//!
//! PR-2 of #360 retired the legacy global-BVH path for `OmeAccel`,
//! the TLAS+BLAS pool acceleration structure. The pool's
//! pre-allocated GPU buffers replace the prior double-buffered
//! `(BVH, leaf_aabbs, RaymarchPayload, SdfPrimitive)` slots: every
//! `insert_chunk` / `refit_chunk` writes into the pre-allocated pool
//! slices via `Queue::write_buffer`, so the bind group references
//! stay stable for the lifetime of the renderer.
//!
//! # Single-chunk migration
//!
//! PR-2 drives `OmeAccel` with **one** chunk (`key = 0`) holding every
//! visible SDF primitive. `update_single_chunk` is the only entry
//! point: it removes + reinserts the lone chunk on scene-hash change
//! and ticks `update_gpu` every frame to upload `tlas_uniforms`.
//! PR-3 expands this to per-chunk bucketing via `ChunkManager` without
//! touching the renderer pipeline.
//!
//! # Smoothness
//!
//! Per-primitive smoothness lives inside `SdfPrimitive` (byte 44 — the
//! slot the legacy `_pad0` field used to occupy). The pool shader
//! reads `prim.smoothness` directly during the per-role accumulator
//! fold, so the legacy `RaymarchPayload[]` SSBO is gone in this PR.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ome_bvh::{
    AccelBuffers, AccelCaps, AccelError, ChunkInsert, IS_RAYMARCH, LeafAabb, OmeAccel,
    ROLE_RAYMARCH_MASK,
};

use crate::raymarch::instance::SdfPrimitive;

/// Single-chunk `ChunkKey` used for the PR-2 migration. PR-3 drops
/// this constant and grows `update_scene` into a per-chunk bucketer.
const SINGLE_CHUNK_KEY: u64 = 0;

/// Raymarch-side BVH state. Owns one `OmeAccel` plus a CPU-side scene
/// hash so unchanged frames skip the GPU re-upload entirely.
pub struct BvhState {
    accel: OmeAccel,
    /// Hash of `(leaf_aabbs ⊕ primitives)` of the chunk currently
    /// resident at `SINGLE_CHUNK_KEY`. `None` when the pool has no
    /// resident chunk (empty scene or pre-first-frame).
    last_scene_hash: Option<u64>,
    /// Number of primitives currently resident in the lone chunk. `0`
    /// when the scene is empty.
    primitive_count: u32,
}

impl BvhState {
    /// Build the pool. `primitive_stride` is the byte size of one
    /// primitive in `primitives_pool`; the renderer always passes
    /// `size_of::<SdfPrimitive>()`.
    pub fn new(device: &wgpu::Device) -> Self {
        let accel = OmeAccel::new(
            device,
            AccelCaps::default(),
            std::mem::size_of::<SdfPrimitive>() as u32,
        )
        .expect("OmeAccel::new with default caps must stay within MAX_CHUNKS_LIMIT");
        Self {
            accel,
            last_scene_hash: None,
            primitive_count: 0,
        }
    }

    /// Borrow the pool buffers — the renderer's scene bind group
    /// references these directly. Stable for the lifetime of `self`.
    pub fn buffers(&self) -> &AccelBuffers {
        &self.accel.buffers
    }

    /// Number of primitives currently resident in the lone chunk. `0`
    /// before any scene resolves or when the scene goes empty.
    pub fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    /// Drive the pool with a single chunk holding every visible SDF
    /// primitive. Skips the GPU re-upload when the scene hash matches
    /// the previous frame; always ticks `update_gpu` so
    /// `tlas_uniforms.k_*_global` track the per-frame reduce.
    ///
    /// Empty scenes (`leaf_aabbs.is_empty()`) evict the chunk if one
    /// is resident so subsequent frames see `tlas_uniforms.num_chunks == 0`.
    pub fn update_single_chunk(
        &mut self,
        queue: &wgpu::Queue,
        leaf_aabbs: &[LeafAabb],
        primitives: &[SdfPrimitive],
        max_smoothness_radius: f32,
        k_int_global: f32,
        k_sub_global: f32,
    ) -> Result<(), AccelError> {
        debug_assert_eq!(
            leaf_aabbs.len(),
            primitives.len(),
            "leaf_aabbs and primitives must align 1:1 — one entry per primitive",
        );

        if leaf_aabbs.is_empty() {
            // Empty scene — evict if needed, then upload uniforms so
            // the GPU sees `num_chunks == 0`.
            if self.last_scene_hash.is_some() {
                let _ = self.accel.remove_chunk(queue, SINGLE_CHUNK_KEY);
                self.last_scene_hash = None;
                self.primitive_count = 0;
            }
            self.accel.update_gpu(queue, k_int_global, k_sub_global);
            return Ok(());
        }

        let scene_hash = Self::hash_scene(leaf_aabbs, primitives);
        if Some(scene_hash) != self.last_scene_hash {
            // PR-2 always rebuilds on hash change. PR-3 will switch to
            // `refit_chunk` when cardinality is preserved (entity
            // movement) and only fall back to remove+insert on
            // primitive add/remove.
            if self.last_scene_hash.is_some() {
                let _ = self.accel.remove_chunk(queue, SINGLE_CHUNK_KEY);
            }
            let primitives_bytes: &[u8] = bytemuck::cast_slice(primitives);
            self.accel.insert_chunk(
                queue,
                ChunkInsert {
                    key: SINGLE_CHUNK_KEY,
                    leaf_aabbs,
                    primitives_bytes,
                    max_smoothness_radius,
                },
            )?;
            self.last_scene_hash = Some(scene_hash);
            self.primitive_count = leaf_aabbs.len() as u32;
        }

        self.accel.update_gpu(queue, k_int_global, k_sub_global);
        Ok(())
    }

    /// Stable hash over the bytes the WGSL traversal reads. Mirrors
    /// the legacy `BvhState::hash_scene` shape: a smoothness or
    /// rotation change with no AABB delta must still trigger a
    /// re-upload, otherwise the pool keeps stale data and the rendered
    /// output silently drifts.
    fn hash_scene(leaf_aabbs: &[LeafAabb], primitives: &[SdfPrimitive]) -> u64 {
        let mut h = DefaultHasher::new();
        leaf_aabbs.len().hash(&mut h);
        for la in leaf_aabbs {
            la.flags.hash(&mut h);
            la.entity_id.hash(&mut h);
            for c in la.aabb_min.iter().chain(la.aabb_max.iter()) {
                c.to_bits().hash(&mut h);
            }
        }
        primitives.len().hash(&mut h);
        for p in primitives {
            p.type_tag.hash(&mut h);
            p.smoothness.to_bits().hash(&mut h);
            for c in p
                .position
                .iter()
                .chain(p.rotation.iter())
                .chain(p.scale.iter())
                .chain(p.params.iter())
            {
                c.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }
}

/// Compute the scene-wide per-role smoothness maxima (`k_int_global`,
/// `k_sub_global`, `max_smoothness_radius`) the pool needs in
/// `tlas_uniforms` + the chunk descriptor inflation. Folds over
/// `(leaf_flags, primitive_smoothness)` pairs so the pool layer stays
/// independent of the consumer's `BlendInfo` / `SdfBlend` types.
///
/// Currently only consumed by the unit tests; PR-3 wires `update_scene`
/// onto this helper when per-chunk bucketing lands and the per-frame
/// reduce moves out of `update.rs`'s second pass.
///
/// Returns `(k_int_global, k_sub_global, max_smoothness_radius)`.
#[allow(dead_code)]
pub(super) fn reduce_per_role_smoothness(
    leaf_aabbs: &[LeafAabb],
    primitives: &[SdfPrimitive],
) -> (f32, f32, f32) {
    let mut k_add = 0.0f32;
    let mut k_int = 0.0f32;
    let mut k_sub = 0.0f32;
    for (la, prim) in leaf_aabbs.iter().zip(primitives.iter()) {
        if la.flags & IS_RAYMARCH == 0 {
            continue;
        }
        let role = la.flags & ROLE_RAYMARCH_MASK;
        match role {
            1 => k_int = k_int.max(prim.smoothness),
            2 => k_sub = k_sub.max(prim.smoothness),
            _ => k_add = k_add.max(prim.smoothness),
        }
    }
    let envelope = k_add.max(k_int).max(k_sub);
    (k_int, k_sub, envelope)
}
