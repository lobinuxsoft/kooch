//! [`LeafAabb`] — multi-consumer per-primitive leaf metadata uploaded
//! alongside the BVH itself.
//!
//! Lives in `ome_bvh` (rather than the consuming crates) because every
//! BVH consumer — raymarch, physics broadphase, frustum culling, light
//! culling — reads the same buffer through the same WGSL traversal
//! library. The flag scheme below is the contract: each consumer
//! filters by its own `IS_*` bit during traversal; role-mask bits drive
//! the raymarch's per-role accumulator only.
//!
//! See `oh_my_engine/docs/book/architecture/bvh-multi-consumer.md`
//! (S8 of #115 PR-5) for the full envelope rule and per-bit ownership
//! conventions.

use bytemuck::{Pod, Zeroable};

/// Bits 0-1 of [`LeafAabb::flags`] — raymarch CSG role. Only meaningful
/// when [`IS_RAYMARCH`] is set; ignored by every other consumer.
#[allow(dead_code)]
pub const ROLE_RAYMARCH_MASK: u32 = 0x3;
pub const ROLE_RAYMARCH_ADD: u32 = 0x0;
pub const ROLE_RAYMARCH_INT: u32 = 0x1;
pub const ROLE_RAYMARCH_SUB: u32 = 0x2;

/// Bit 2 — leaf participates in the raymarch SDF traversal. The
/// raymarch shader skips leaves with this bit clear; physics-only or
/// mesh-only entities can therefore share the same BVH without ever
/// being evaluated as SDF primitives.
pub const IS_RAYMARCH: u32 = 1 << 2;
/// Bit 3 — leaf participates in physics broadphase (#42, S4 of PR-5).
pub const IS_COLLIDER: u32 = 1 << 3;
/// Bit 4 — leaf participates in frustum / occlusion culling (#91, S5).
pub const IS_VISIBLE_MESH: u32 = 1 << 4;
/// Bit 5 — reserved for the light-culling consumer (#27). Defined here
/// so no future consumer accidentally claims the bit; not yet read by
/// any traversal — fence the issue before consuming it.
#[allow(dead_code)]
pub const IS_LIGHT: u32 = 1 << 5;

/// Per-leaf metadata uploaded to the GPU alongside the BVH itself.
///
/// **32 bytes, std430-clean** — same layout family as
/// [`crate::BvhNode`], so the WGSL traversal library can read both
/// buffers without per-vendor alignment fixes. Field order mirrors the
/// WGSL `LeafAabb`:
///
/// ```text
///   [0..12 ) aabb_min:   vec3<f32>
///   [12..16) flags:      u32
///   [16..28) aabb_max:   vec3<f32>
///   [28..32) entity_id:  u32
/// ```
///
/// `flags` packs role + consumer membership; see the `IS_*` /
/// `ROLE_RAYMARCH_*` constants on this module. `entity_id` is the ECS
/// entity index broadphase / frustum cull use to return entity-keyed
/// pair lists and visibility sets — raymarch ignores this field.
///
/// Consumer-specific side payloads (e.g. the raymarch's per-primitive
/// smoothness) live in **separate** storage buffers maintained by the
/// consuming crate, not here. See `ome_render::raymarch::instance::
/// RaymarchPayload` for an example.
///
/// `aabb_min` / `aabb_max` are the world-space bounds **already
/// inflated** by the per-role envelope (currently the per-role smooth-
/// blend `k_max`). The "tighter per-role AABBs" follow-up tracked in
/// the S7 bench of PR-5 may split this into per-consumer envelopes if
/// broadphase false-positive ratio justifies it.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug)]
pub struct LeafAabb {
    pub aabb_min: [f32; 3],
    pub flags: u32,
    pub aabb_max: [f32; 3],
    pub entity_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_aabb_layout_is_32_bytes() {
        assert_eq!(std::mem::size_of::<LeafAabb>(), 32);
        assert_eq!(std::mem::align_of::<LeafAabb>(), 4);
    }

    #[test]
    fn leaf_aabb_field_offsets_match_wgsl() {
        use std::mem::offset_of;
        // Mirror the std430 layout the WGSL traversal expects.
        assert_eq!(offset_of!(LeafAabb, aabb_min), 0);
        assert_eq!(offset_of!(LeafAabb, flags), 12);
        assert_eq!(offset_of!(LeafAabb, aabb_max), 16);
        assert_eq!(offset_of!(LeafAabb, entity_id), 28);
    }

    #[test]
    fn flag_bits_distinct_and_packed() {
        // Roles claim bits 0-1, IS_RAYMARCH gates them on bit 2.
        // Each consumer flag (collider / visible / light) sits in a
        // distinct bit so a single u32 can mark the same leaf as
        // belonging to multiple consumers simultaneously.
        assert_eq!(ROLE_RAYMARCH_MASK, 0b0011);
        assert_eq!(IS_RAYMARCH, 0b0100);
        assert_eq!(IS_COLLIDER, 0b1000);
        assert_eq!(IS_VISIBLE_MESH, 0b1_0000);
        assert_eq!(IS_LIGHT, 0b10_0000);
        assert_ne!(ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_INT);
        assert_ne!(ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_SUB);
        assert_ne!(ROLE_RAYMARCH_INT, ROLE_RAYMARCH_SUB);
        let consumers = IS_RAYMARCH | IS_COLLIDER | IS_VISIBLE_MESH | IS_LIGHT;
        assert_eq!(ROLE_RAYMARCH_MASK & consumers, 0);
    }
}
