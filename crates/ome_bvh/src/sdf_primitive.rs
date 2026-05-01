//! GPU-bound SDF primitive POD. Lives next to [`crate::leaf::LeafAabb`]
//! because both are read by the WGSL traversal and both need to be
//! constructible from any consumer crate without pulling in `ome_render`.
//!
//! # Why here
//!
//! The pool's primitive byte stride is `size_of::<SdfPrimitive>()`.
//! `OmeAccel` accepts opaque `primitives_bytes: &[u8]`, but every
//! producer (`ome_render`'s ECS scene collector, `ome_world`'s
//! [`crate::sdf_primitive`] content sources, the future Edit Baker)
//! needs to lay bytes out the same way. Hoisting the type up here
//! removes the renderer from that chain.
//!
//! # WGSL contract
//!
//! Field offsets match `raymarch_main.wgsl::SdfPrimitive` byte-for-byte:
//! - `position` (vec3 at 0) + `type_tag` (u32 at 12) fill the first 16 B slot.
//! - `rotation` (vec4 at 16) is naturally 16-aligned.
//! - `scale` (vec3 at 32) + `smoothness` (f32 at 44) fill the next 16 B slot.
//! - `params` (vec4 at 48) holds primitive-specific data; interpretation
//!   depends on `type_tag`. Closes the struct at 64 B (multiple of 16).
//!
//! `smoothness` lives in the slot the legacy `_pad0` occupied (#360 PR-2).
//! The pool-driven shader reads `prim.smoothness` directly during the
//! per-role accumulator fold.

use bytemuck::{Pod, Zeroable};

/// Primitive type tags. Must match the `switch` in
/// `raymarch_main.wgsl::eval_primitive`.
pub const TYPE_SPHERE: u32 = 0;
pub const TYPE_BOX: u32 = 1;
pub const TYPE_CAPSULE: u32 = 2;
pub const TYPE_CYLINDER: u32 = 3;
pub const TYPE_TORUS: u32 = 4;
pub const TYPE_PLANE: u32 = 5;

/// Per-entity SDF primitive (64 bytes). See module docstring for the
/// WGSL contract pinned to this layout.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct SdfPrimitive {
    pub position: [f32; 3],
    pub type_tag: u32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub smoothness: f32,
    pub params: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdf_primitive_layout_is_64_bytes() {
        assert_eq!(std::mem::size_of::<SdfPrimitive>(), 64);
        assert_eq!(std::mem::align_of::<SdfPrimitive>(), 4);
    }

    #[test]
    fn sdf_primitive_field_offsets_match_wgsl() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(SdfPrimitive, position), 0);
        assert_eq!(offset_of!(SdfPrimitive, type_tag), 12);
        assert_eq!(offset_of!(SdfPrimitive, rotation), 16);
        assert_eq!(offset_of!(SdfPrimitive, scale), 32);
        assert_eq!(offset_of!(SdfPrimitive, smoothness), 44);
        assert_eq!(offset_of!(SdfPrimitive, params), 48);
    }
}
