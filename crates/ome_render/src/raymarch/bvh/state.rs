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
use ome_world::{ChunkContent, ChunkId};

use crate::raymarch::instance::SdfPrimitive;

/// `ChunkKey` reserved for the legacy ECS-driven scene path. Streaming
/// chunks set bit 63 in their key, so this stays disjoint from any
/// streaming key by construction.
const SINGLE_CHUNK_KEY: u64 = 0;

/// Bits per coord in [`chunk_id_to_key`]. 20 bits → ±524 288 chunks
/// per axis at level 0 = ±33 M m radius, well past the planet-scale
/// envelope the streaming layer is sized for.
const STREAMING_KEY_COORD_BITS: u32 = 20;
const STREAMING_KEY_COORD_MASK: u64 = (1u64 << STREAMING_KEY_COORD_BITS) - 1;
const STREAMING_KEY_LEVEL_MASK: u64 = 0xF;
const STREAMING_KEY_FLAG: u64 = 1u64 << 63;

/// Bijective bit-pack of a [`ChunkId`] into the pool's `ChunkKey` (a
/// `u64`). Bit 63 is forced to 1 so streaming chunks land in a key
/// space disjoint from [`SINGLE_CHUNK_KEY`] = 0 — the legacy ECS
/// single-chunk path keeps its `key = 0` slot regardless of how many
/// streaming chunks coexist with it.
pub(super) fn chunk_id_to_key(id: ChunkId) -> u64 {
    let x = (id.coords.x as i64 as u64) & STREAMING_KEY_COORD_MASK;
    let y = (id.coords.y as i64 as u64) & STREAMING_KEY_COORD_MASK;
    let z = (id.coords.z as i64 as u64) & STREAMING_KEY_COORD_MASK;
    let lvl = (id.level as u64) & STREAMING_KEY_LEVEL_MASK;
    x | (y << STREAMING_KEY_COORD_BITS)
        | (z << (STREAMING_KEY_COORD_BITS * 2))
        | (lvl << (STREAMING_KEY_COORD_BITS * 3))
        | STREAMING_KEY_FLAG
}

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
    /// Total leaf count across **streaming** chunks (excludes the
    /// legacy single-chunk key 0). Maintained by
    /// [`Self::insert_streaming_chunk`] / [`Self::remove_streaming_chunk`]
    /// so the renderer's `bvh_n` "scene-empty" marker stays accurate
    /// in the multi-chunk world.
    streaming_primitive_count: u32,
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
            streaming_primitive_count: 0,
        }
    }

    /// Borrow the pool buffers — the renderer's scene bind group
    /// references these directly. Stable for the lifetime of `self`.
    pub fn buffers(&self) -> &AccelBuffers {
        &self.accel.buffers
    }

    /// Number of primitives currently resident in the lone chunk. `0`
    /// before any scene resolves or when the scene goes empty.
    /// Public-API accessor; consumers in test harnesses + future
    /// telemetry hooks read this even though no current call site
    /// inside the crate does.
    #[allow(dead_code)]
    pub fn primitive_count(&self) -> u32 {
        self.primitive_count
    }

    /// Sum of primitive counts across the legacy single chunk + every
    /// streaming chunk. Drives the renderer's `SceneMeta.bvh_n` field
    /// — `0` here means "no scene", regardless of which path
    /// produced the primitives.
    pub fn total_primitive_count(&self) -> u32 {
        self.primitive_count
            .saturating_add(self.streaming_primitive_count)
    }

    /// Number of live streaming chunks currently resident in the pool
    /// (excludes the legacy single-chunk slot). Used by the integration
    /// test to assert the streaming flow round-tripped end-to-end.
    #[allow(dead_code)]
    pub fn streaming_chunk_count(&self) -> u32 {
        self.accel.live_chunk_count().saturating_sub(
            if self.last_scene_hash.is_some() { 1 } else { 0 },
        )
    }

    /// Look up whether a streaming chunk for `id` is currently resident
    /// in the pool. Used by the integration test + by the editor's
    /// streaming HUD when one lands.
    #[allow(dead_code)]
    pub fn has_streaming_chunk(&self, id: ChunkId) -> bool {
        self.accel.lookup(chunk_id_to_key(id)).is_some()
    }

    /// Bring a streaming chunk into the pool. Empty content is a
    /// no-op (the pool rejects `EmptyPrimitives`); the streaming layer
    /// already filters those before calling here, but the guard keeps
    /// the invariant local to this method.
    ///
    /// Idempotent on repeat insertion: if the chunk is already
    /// resident, the call is a no-op — process_queues' dedup makes
    /// repeats unlikely, but this lets the renderer drain without
    /// having to track its own resident-set mirror.
    pub fn insert_streaming_chunk(
        &mut self,
        queue: &wgpu::Queue,
        chunk_id: ChunkId,
        content: &ChunkContent,
    ) -> Result<(), AccelError> {
        if content.is_empty() {
            return Ok(());
        }
        let key = chunk_id_to_key(chunk_id);
        if self.accel.lookup(key).is_some() {
            return Ok(());
        }
        let primitives_bytes: &[u8] = bytemuck::cast_slice(&content.primitives);
        self.accel.insert_chunk(
            queue,
            ChunkInsert {
                key,
                leaf_aabbs: &content.leaf_aabbs,
                primitives_bytes,
                max_smoothness_radius: content.max_smoothness_radius,
            },
        )?;
        self.streaming_primitive_count = self
            .streaming_primitive_count
            .saturating_add(content.primitives.len() as u32);
        Ok(())
    }

    /// Evict a streaming chunk. No-op when the chunk is not currently
    /// resident — the streaming layer may double-fire on chunks that
    /// loaded and unloaded inside the same `process_queues` budget,
    /// and the renderer doesn't need to filter those before calling.
    pub fn remove_streaming_chunk(
        &mut self,
        queue: &wgpu::Queue,
        chunk_id: ChunkId,
    ) -> Result<(), AccelError> {
        let key = chunk_id_to_key(chunk_id);
        let Some(handle) = self.accel.lookup(key) else {
            return Ok(());
        };
        let leaves = self
            .accel
            .descriptor(handle)
            .map(|d| d.primitive_count)
            .unwrap_or(0);
        self.accel.remove_chunk(queue, key)?;
        self.streaming_primitive_count =
            self.streaming_primitive_count.saturating_sub(leaves);
        Ok(())
    }

    /// Tick `tlas_uniforms` after a streaming insert/remove batch so
    /// the next traversal sees the updated TLAS topology + per-frame
    /// `k_*_global` reduce. The legacy single-chunk path calls this
    /// from inside [`Self::update_single_chunk`]; streaming callers
    /// drive it explicitly because their `update_scene` already owns
    /// the per-frame reduce values.
    #[allow(dead_code)]
    pub fn tick_streaming(&mut self, queue: &wgpu::Queue, k_int_global: f32, k_sub_global: f32) {
        self.accel.update_gpu(queue, k_int_global, k_sub_global);
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
        encoder: &mut wgpu::CommandEncoder,
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
            self.accel.update_gpu(queue, encoder, k_int_global, k_sub_global);
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

        self.accel.update_gpu(queue, encoder, k_int_global, k_sub_global);
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
