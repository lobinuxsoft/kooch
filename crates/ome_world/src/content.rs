//! [`ChunkContentSource`] — load-side counterpart to
//! [`crate::manager::ChunkEvictionListener`].
//!
//! When `ChunkManager` transitions a chunk `Unloaded → Loaded`, the
//! registered content source materialises the chunk's volume primitives
//! plus their per-leaf AABBs. The streaming layer then forwards that
//! content to the GPU pool (`ome_bvh::OmeAccel::insert_chunk`) without
//! ever crossing the renderer crate boundary.
//!
//! # DOD discipline
//!
//! [`ChunkContent`] is plain old data: `Vec<VolumePrimitive>` (each entry
//! 64 B `#[repr(C)]` POD), `Vec<LeafAabb>` (each 32 B `#[repr(C)]` POD)
//! and a single `f32` envelope. No pointer indirection, no `dyn`
//! payload, no per-primitive heap allocation. The renderer uploads
//! these slices to the pool via the existing `Queue::write_buffer`
//! path — same bytes, no marshalling.
//!
//! [`ChunkContentSource`] is the **single** `dyn` boundary on the load
//! side, mirroring [`ChunkEvictionListener`] on the unload side. CPU-
//! only by construction: the trait never sees a `wgpu::Device` /
//! `Queue` / `BvhState`. GPU upload is the streaming layer's job.

use ome_bvh::{Aabb, LeafAabb, volume_primitive::VolumePrimitive};

use crate::chunk::ChunkId;

/// Output of one [`ChunkContentSource::populate`] call. Owned because
/// the streaming layer hands the slices straight to the GPU writer
/// without further copying — the borrow checker would otherwise force
/// a per-frame round-trip through a temporary scratch buffer.
///
/// `primitives.len() == leaf_aabbs.len()` is a structural invariant —
/// each primitive owns exactly one leaf AABB. `OmeAccel::insert_chunk`
/// debug-asserts the same equality at the pool boundary.
#[derive(Clone, Debug, Default)]
pub struct ChunkContent {
    /// 1:1 with `leaf_aabbs`. `bytemuck::cast_slice` to `&[u8]` for the
    /// `OmeAccel::insert_chunk` upload — the pool never sees the typed
    /// view, only the byte stride.
    pub primitives: Vec<VolumePrimitive>,
    /// Per-primitive leaf AABBs already inflated by the per-role
    /// smooth-blend envelope. `flags` carries `IS_RAYMARCH | role`;
    /// `entity_id` is `0` for procedurally-spawned content (no ECS
    /// entity backing it).
    pub leaf_aabbs: Vec<LeafAabb>,
    /// Conservative per-chunk envelope = `max(k_add, k_int, k_sub)`
    /// over this chunk's primitives. `OmeAccel::insert_chunk` inflates
    /// the chunk descriptor's AABB by this radius so cross-chunk smooth
    /// blends stay conservative under the TLAS cull.
    pub max_smoothness_radius: f32,
}

impl ChunkContent {
    /// Empty content — no primitives. Streaming layer treats this as a
    /// no-op (does not call `OmeAccel::insert_chunk`, since the pool
    /// rejects `EmptyPrimitives`).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of primitives in this content. `0` for an empty chunk.
    pub fn len(&self) -> usize {
        debug_assert_eq!(
            self.primitives.len(),
            self.leaf_aabbs.len(),
            "ChunkContent invariant: primitives and leaf_aabbs must align 1:1",
        );
        self.primitives.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Source that materialises a chunk's volume primitives at load time.
///
/// Implementations must be:
/// - **Pure** w.r.t. `(chunk_id, world_aabb)`: same input → byte-identical
///   output. The pool's TLAS topology is determinism-sensitive (AC6 of
///   #360); a non-pure source would silently break it.
/// - **Send + Sync**: the streaming layer holds the source behind a
///   `Box<dyn>` and a future async loader will populate from a worker
///   thread without any further synchronisation hooks.
/// - **CPU-only**: no `wgpu` types. GPU upload is the streaming layer's
///   responsibility.
///
/// One source per `ChunkManager` for now; multi-source merge / priority
/// is intentionally out of scope (see the `#363` issue body).
pub trait ChunkContentSource: Send + Sync {
    /// Materialise the content for one chunk. Returning
    /// [`ChunkContent::empty`] is the canonical "no content here" answer
    /// — the streaming layer treats it as a load completed with zero
    /// primitives, NOT an error.
    fn populate(&self, chunk_id: ChunkId, world_aabb: Aabb) -> ChunkContent;
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{IVec3, Vec3};
    use ome_bvh::{IS_RAYMARCH, ROLE_RAYMARCH_ADD, volume_primitive::TYPE_SPHERE};

    struct EmptySource;
    impl ChunkContentSource for EmptySource {
        fn populate(&self, _id: ChunkId, _aabb: Aabb) -> ChunkContent {
            ChunkContent::empty()
        }
    }

    struct OneSphereSource;
    impl ChunkContentSource for OneSphereSource {
        fn populate(&self, _id: ChunkId, _aabb: Aabb) -> ChunkContent {
            let prim = VolumePrimitive {
                position: [0.0; 3],
                type_tag: TYPE_SPHERE,
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
                smoothness: 0.0,
                params: [1.0, 0.0, 0.0, 0.0],
            };
            ChunkContent {
                primitives: vec![prim],
                leaf_aabbs: vec![LeafAabb {
                    aabb_min: [-1.0; 3],
                    flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
                    aabb_max: [1.0; 3],
                    entity_id: 0,
                }],
                max_smoothness_radius: 0.0,
            }
        }
    }

    fn id(x: i32) -> ChunkId {
        ChunkId::new(IVec3::new(x, 0, 0), 0)
    }

    fn aabb() -> Aabb {
        Aabb::new(Vec3::ZERO, Vec3::splat(64.0))
    }

    #[test]
    fn empty_content_is_zero_length() {
        assert!(ChunkContent::empty().is_empty());
        assert_eq!(ChunkContent::empty().len(), 0);
    }

    #[test]
    fn boxed_dyn_source_dispatches() {
        let src: Box<dyn ChunkContentSource> = Box::new(OneSphereSource);
        let content = src.populate(id(0), aabb());
        assert_eq!(content.len(), 1);
        assert!((content.primitives[0].params[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_source_returns_empty_content() {
        let src: Box<dyn ChunkContentSource> = Box::new(EmptySource);
        assert!(src.populate(id(7), aabb()).is_empty());
    }
}
