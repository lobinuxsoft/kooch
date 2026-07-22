//! [`VolumePrimitive`] — an analytic shape instance, POD.
//!
//! A transform plus a type tag plus per-type parameters: a sphere, box,
//! capsule, cylinder, torus or plane placed in the world. Lives next to
//! [`crate::leaf::LeafAabb`] because both describe what a BVH leaf
//! points at, and both must be constructible from any consumer crate
//! without pulling in `ome_render`.
//!
//! Named for what it is rather than how it used to be evaluated. It was
//! `SdfPrimitive`, back when a ray-marcher folded these into a signed
//! distance field; that renderer was deleted in the 2026-05-02 pivot and
//! the brush components followed it in 2026-07. The type outlived both
//! because `ome_world` uses it as chunk content — the procedural city
//! source emits boxes and cylinders, and the BVH indexes them.
//!
//! # Layout
//!
//! `#[repr(C)]` + `Pod`, 64 bytes, every field 16-byte-slot aligned:
//! `position` (vec3 at 0) + `type_tag` (u32 at 12); `rotation` (vec4 at
//! 16); `scale` (vec3 at 32) + `smoothness` (f32 at 44); `params`
//! (vec4 at 48) holding per-type data.
//!
//! That layout was pinned to a WGSL struct in `raymarch_main.wgsl`,
//! which no longer exists — **no shader reads this type today**. The
//! guarantee is kept because the pool hands `primitives_bytes: &[u8]`
//! to `OmeAccel` and every producer must agree on the stride, and
//! because a future GPU consumer would want exactly this shape. The
//! offset tests below are what keeps that honest.
//!
//! `smoothness` is the blend radius the old evaluator used to soften
//! unions; it survives as the inflation applied to a leaf AABB, so
//! culling still bounds whatever a future evaluator does with it.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Quat, Vec3};

use crate::aabb::Aabb;

/// Primitive type tags. A GPU consumer must switch on these in the same
/// order; nothing does today.
pub const TYPE_SPHERE: u32 = 0;
pub const TYPE_BOX: u32 = 1;
pub const TYPE_CAPSULE: u32 = 2;
pub const TYPE_CYLINDER: u32 = 3;
pub const TYPE_TORUS: u32 = 4;
pub const TYPE_PLANE: u32 = 5;

/// An analytic shape instance (64 bytes). See the module docs for the
/// layout guarantee and why it is kept.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct VolumePrimitive {
    pub position: [f32; 3],
    pub type_tag: u32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub smoothness: f32,
    pub params: [f32; 4],
}

/// Half-extent assigned to plane primitives in every axis. Large enough
/// that any ray that reaches the BVH root will visit the plane leaf,
/// regardless of orientation. Not `f32::INFINITY` — that produces NaNs
/// in the slab test when a ray is exactly axis-aligned with the plane.
pub const PLANE_HALF_EXTENT: f32 = 1.0e10;

/// World-space AABB of a primitive instance, inflated by `inflation` to
/// absorb smooth-blend support.
///
/// Shared by every producer of leaf AABBs so a given primitive always
/// bounds the same way — a culler and an evaluator that disagree drop
/// silhouettes.
///
/// The world-space AABB is built by:
/// 1. Reading per-type half-extents in local space.
/// 2. Multiplying by `|scale|` per axis.
/// 3. Rotating the box by the quaternion (use `abs(rot_matrix)` on the
///    half-extent vector — classical OBB→AABB enclosing formula).
/// 4. Translating by `position`.
/// 5. Inflating by `max(inflation, 0)` per axis.
pub fn primitive_aabb(prim: &VolumePrimitive, inflation: f32) -> Aabb {
    let position = Vec3::from_array(prim.position);

    if prim.type_tag == TYPE_PLANE {
        // Rotation does not change "covers everything" semantics —
        // `PLANE_HALF_EXTENT` already dominates any rotation envelope.
        return Aabb::from_centre(position, Vec3::splat(PLANE_HALF_EXTENT));
    }

    let local_half = local_half_extents(prim);
    let scale = Vec3::from_array(prim.scale).abs();
    let scaled = local_half * scale;

    let q = Quat::from_array(prim.rotation);
    let m = Mat3::from_quat(q);
    let abs_m = Mat3::from_cols(m.x_axis.abs(), m.y_axis.abs(), m.z_axis.abs());
    let rotated_half = abs_m * scaled;

    let inflated_half = rotated_half + Vec3::splat(inflation.max(0.0));
    Aabb::from_centre(position, inflated_half)
}

fn local_half_extents(prim: &VolumePrimitive) -> Vec3 {
    match prim.type_tag {
        TYPE_SPHERE => Vec3::splat(prim.params[0]),
        TYPE_BOX => {
            let r = prim.params[3].max(0.0);
            Vec3::new(prim.params[0], prim.params[1], prim.params[2]) + Vec3::splat(r)
        }
        TYPE_CAPSULE => {
            let h = prim.params[0];
            let r = prim.params[1];
            Vec3::new(r, h + r, r)
        }
        TYPE_CYLINDER => {
            let h = prim.params[0];
            let r = prim.params[1];
            Vec3::new(r, h, r)
        }
        TYPE_TORUS => {
            let major = prim.params[0];
            let minor = prim.params[1];
            Vec3::new(major + minor, minor, major + minor)
        }
        _ => Vec3::splat(PLANE_HALF_EXTENT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    const EPS: f32 = 1e-4;

    #[test]
    fn volume_primitive_layout_is_64_bytes() {
        assert_eq!(std::mem::size_of::<VolumePrimitive>(), 64);
        assert_eq!(std::mem::align_of::<VolumePrimitive>(), 4);
    }

    #[test]
    fn volume_primitive_field_offsets_match_wgsl() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(VolumePrimitive, position), 0);
        assert_eq!(offset_of!(VolumePrimitive, type_tag), 12);
        assert_eq!(offset_of!(VolumePrimitive, rotation), 16);
        assert_eq!(offset_of!(VolumePrimitive, scale), 32);
        assert_eq!(offset_of!(VolumePrimitive, smoothness), 44);
        assert_eq!(offset_of!(VolumePrimitive, params), 48);
    }

    fn prim(
        type_tag: u32,
        position: [f32; 3],
        rotation: Quat,
        scale: [f32; 3],
        params: [f32; 4],
    ) -> VolumePrimitive {
        VolumePrimitive {
            position,
            type_tag,
            rotation: rotation.to_array(),
            scale,
            smoothness: 0.0,
            params,
        }
    }

    fn approx_aabb(actual: Aabb, expected_min: Vec3, expected_max: Vec3) {
        assert!(
            (actual.min - expected_min).length() < EPS,
            "min: actual={:?}, expected={:?}",
            actual.min,
            expected_min,
        );
        assert!(
            (actual.max - expected_max).length() < EPS,
            "max: actual={:?}, expected={:?}",
            actual.max,
            expected_max,
        );
    }

    #[test]
    fn sphere_canonical() {
        let p = prim(
            TYPE_SPHERE,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [2.5, 0.0, 0.0, 0.0],
        );
        approx_aabb(primitive_aabb(&p, 0.0), Vec3::splat(-2.5), Vec3::splat(2.5));
    }

    #[test]
    fn sphere_translated_and_inflated() {
        let p = prim(
            TYPE_SPHERE,
            [10.0, -3.0, 5.0],
            Quat::IDENTITY,
            [1.0; 3],
            [1.0, 0.0, 0.0, 0.0],
        );
        approx_aabb(
            primitive_aabb(&p, 0.5),
            Vec3::new(8.5, -4.5, 3.5),
            Vec3::new(11.5, -1.5, 6.5),
        );
    }

    #[test]
    fn box_with_rounding() {
        let p = prim(
            TYPE_BOX,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [1.0, 2.0, 3.0, 0.1],
        );
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-1.1, -2.1, -3.1),
            Vec3::new(1.1, 2.1, 3.1),
        );
    }

    #[test]
    fn box_rotated_45_in_y() {
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let p = prim(TYPE_BOX, [0.0; 3], q, [1.0; 3], [1.0, 1.0, 1.0, 0.0]);
        let aabb = primitive_aabb(&p, 0.0);
        let expected = std::f32::consts::SQRT_2;
        assert!((aabb.max.x - expected).abs() < EPS);
        assert!((aabb.max.z - expected).abs() < EPS);
        assert!((aabb.max.y - 1.0).abs() < EPS);
    }

    #[test]
    fn capsule_canonical() {
        let p = prim(
            TYPE_CAPSULE,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [2.0, 0.5, 0.0, 0.0],
        );
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-0.5, -2.5, -0.5),
            Vec3::new(0.5, 2.5, 0.5),
        );
    }

    #[test]
    fn cylinder_no_caps() {
        let p = prim(
            TYPE_CYLINDER,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [3.0, 1.0, 0.0, 0.0],
        );
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-1.0, -3.0, -1.0),
            Vec3::new(1.0, 3.0, 1.0),
        );
    }

    #[test]
    fn torus_xz_plane() {
        let p = prim(
            TYPE_TORUS,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [2.0, 0.3, 0.0, 0.0],
        );
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-2.3, -0.3, -2.3),
            Vec3::new(2.3, 0.3, 2.3),
        );
    }

    #[test]
    fn plane_returns_sentinel_extent() {
        let p = prim(
            TYPE_PLANE,
            [0.0, 1.0, 0.0],
            Quat::IDENTITY,
            [1.0; 3],
            [0.0; 4],
        );
        let aabb = primitive_aabb(&p, 0.0);
        assert_eq!(aabb.min, Vec3::splat(-PLANE_HALF_EXTENT) + Vec3::Y);
        assert_eq!(aabb.max, Vec3::splat(PLANE_HALF_EXTENT) + Vec3::Y);
    }

    #[test]
    fn inflation_negative_is_clamped_to_zero() {
        let p = prim(
            TYPE_SPHERE,
            [0.0; 3],
            Quat::IDENTITY,
            [1.0; 3],
            [1.0, 0.0, 0.0, 0.0],
        );
        approx_aabb(
            primitive_aabb(&p, -10.0),
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
        );
    }
}
