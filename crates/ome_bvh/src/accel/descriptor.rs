//! GPU-side descriptors for the TLAS+BLAS pool acceleration structure
//! (issue #360).
//!
//! Both [`ChunkDescriptor`] and [`TlasUniforms`] are `#[repr(C)]` Pod
//! structs with std430-clean layouts. Field offsets are pinned by the
//! `offset_of!` tests at the bottom of this file — any reorder breaks
//! the WGSL contract loudly at `cargo test`.

use bytemuck::{Pod, Zeroable};

/// One entry per resident chunk in the BLAS pool.
///
/// **64 bytes, std430-clean.** Read by the WGSL traversal as
/// `chunk_descriptors[chunk_idx]` after the TLAS descend resolves a
/// leaf to a chunk index. Written CPU-side at insert / refit time.
///
/// `aabb_min` / `aabb_max` are world-space bounds **already inflated**
/// by `max_smoothness_radius` so the TLAS prune stays conservative
/// under cross-chunk smooth-blend bleed (architecture note 3 of the
/// issue body).
///
/// Field order is locked to satisfy the `(vec3, scalar)` 16-byte
/// pairing rule that std430 expects for `vec3<f32>` followed by a
/// scalar — `aabb_min` + `first_node` and `aabb_max` + `node_count`
/// each sit in their own 16-byte slot.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct ChunkDescriptor {
    pub aabb_min: [f32; 3],
    pub first_node: u32,
    pub aabb_max: [f32; 3],
    pub node_count: u32,
    pub first_leaf: u32,
    pub leaf_count: u32,
    pub first_primitive: u32,
    pub primitive_count: u32,
    pub max_smoothness_radius: f32,
    pub _pad: [f32; 3],
}

/// Scene-wide globals consumed by the final per-role combine at the
/// tail of `eval_scene_bvh`.
///
/// **16 bytes, std430-clean.** Reduced CPU-side over the visible chunk
/// set once per frame, uploaded as a uniform buffer.
///
/// `num_chunks` is the count of *live* TLAS leaves (dead-skip slots
/// excluded). `has_intersects` / `has_subs` are non-zero iff at least
/// one chunk in the pool carries a primitive with the corresponding
/// role bit set; the shader skips the matching `smooth_intersection`
/// / `smooth_subtraction` step when the flag is zero so the
/// `±1e6` accumulator identities don't bleed `mix(a, b, t)` precision
/// into the final distance (radv lowers `mix` as `a + (b - a) * t`,
/// which loses the smaller operand at extreme magnitudes — AC2 hit
/// this with diff ≈ 0.03 before the flags landed).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct TlasUniforms {
    pub k_int_global: f32,
    pub k_sub_global: f32,
    pub num_chunks: u32,
    pub has_intersects: u32,
    pub has_subs: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn chunk_descriptor_size_is_64_bytes() {
        assert_eq!(size_of::<ChunkDescriptor>(), 64);
        assert_eq!(align_of::<ChunkDescriptor>(), 4);
    }

    #[test]
    fn chunk_descriptor_field_offsets_match_wgsl() {
        assert_eq!(offset_of!(ChunkDescriptor, aabb_min), 0);
        assert_eq!(offset_of!(ChunkDescriptor, first_node), 12);
        assert_eq!(offset_of!(ChunkDescriptor, aabb_max), 16);
        assert_eq!(offset_of!(ChunkDescriptor, node_count), 28);
        assert_eq!(offset_of!(ChunkDescriptor, first_leaf), 32);
        assert_eq!(offset_of!(ChunkDescriptor, leaf_count), 36);
        assert_eq!(offset_of!(ChunkDescriptor, first_primitive), 40);
        assert_eq!(offset_of!(ChunkDescriptor, primitive_count), 44);
        assert_eq!(offset_of!(ChunkDescriptor, max_smoothness_radius), 48);
        assert_eq!(offset_of!(ChunkDescriptor, _pad), 52);
    }

    #[test]
    fn tlas_uniforms_size_is_32_bytes() {
        assert_eq!(size_of::<TlasUniforms>(), 32);
        assert_eq!(align_of::<TlasUniforms>(), 4);
    }

    #[test]
    fn tlas_uniforms_field_offsets_match_wgsl() {
        assert_eq!(offset_of!(TlasUniforms, k_int_global), 0);
        assert_eq!(offset_of!(TlasUniforms, k_sub_global), 4);
        assert_eq!(offset_of!(TlasUniforms, num_chunks), 8);
        assert_eq!(offset_of!(TlasUniforms, has_intersects), 12);
        assert_eq!(offset_of!(TlasUniforms, has_subs), 16);
        assert_eq!(offset_of!(TlasUniforms, _pad0), 20);
    }

    #[test]
    fn chunk_descriptor_bytemuck_round_trip() {
        let d = ChunkDescriptor {
            aabb_min: [1.0, 2.0, 3.0],
            first_node: 100,
            aabb_max: [4.0, 5.0, 6.0],
            node_count: 50,
            first_leaf: 200,
            leaf_count: 25,
            first_primitive: 300,
            primitive_count: 25,
            max_smoothness_radius: 0.5,
            _pad: [0.0; 3],
        };
        let bytes = bytemuck::bytes_of(&d);
        assert_eq!(bytes.len(), 64);
        let back: &ChunkDescriptor = bytemuck::from_bytes(bytes);
        assert_eq!(*back, d);
    }
}
