//! World-space AABB computation for every SDF primitive type.
//!
//! Used by the raymarch BVH integration (PR-4 of #115) to feed the GPU
//! LBVH builder. Each primitive's local-space half-extents are scaled
//! by the entity's `scale`, rotated by its quaternion, translated to
//! world space, and inflated by a per-instance margin that absorbs the
//! smooth-blend support radius (see [`primitive_aabb`] docs).
//!
//! Planes are special-cased: their analytic AABB is infinite in two
//! axes, so we return a sentinel half-extent of `PLANE_HALF_EXTENT`
//! (1e10) instead. The BVH treats them as "covers everything" — every
//! ray query will visit them, matching the plane SDF's semantics.

use glam::{Mat3, Quat, Vec3};
use ome_bvh::Aabb;

use super::instance::{
    SdfPrimitive, TYPE_BOX, TYPE_CAPSULE, TYPE_CYLINDER, TYPE_PLANE, TYPE_SPHERE, TYPE_TORUS,
};

/// Half-extent assigned to plane primitives in every axis. Large enough
/// that any ray that reaches the BVH root will visit the plane leaf,
/// regardless of orientation. Not `f32::INFINITY` — that produces NaNs
/// in the slab test when a ray is exactly axis-aligned with the plane.
pub(crate) const PLANE_HALF_EXTENT: f32 = 1.0e10;

/// World-space AABB of an SDF primitive instance, inflated by
/// `inflation` to absorb smooth-blend support.
///
/// `inflation` is the smooth-blend radius the primitive participates in.
/// Smooth-union / smooth-intersect / smooth-subtract operators have
/// support that decays exponentially with distance from the surface,
/// vanishingly small beyond ~`4 × k`. Inflating by `k` itself is the
/// minimum to keep the BVH cull conservative; downstream code may use
/// a larger margin if it accumulates smoothness across the CSG tree.
///
/// The world-space AABB is built by:
/// 1. Reading per-type half-extents in local space.
/// 2. Multiplying by `|scale|` per axis.
/// 3. Rotating the box by the quaternion (use `abs(rot_matrix)` on the
///    half-extent vector — classical OBB→AABB enclosing formula).
/// 4. Translating by `position`.
/// 5. Inflating by `max(inflation, 0)` per axis.
pub fn primitive_aabb(prim: &SdfPrimitive, inflation: f32) -> Aabb {
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

/// Per-type local-space half-extents (canonical orientation, unit
/// scale). Mirrors the SDF analytic shape encoded in
/// `crates/ome_sdf/shaders/sdf_primitives.wgsl`.
fn local_half_extents(prim: &SdfPrimitive) -> Vec3 {
    match prim.type_tag {
        TYPE_SPHERE => Vec3::splat(prim.params[0]),
        TYPE_BOX => {
            // params.xyz = half-extents, params.w = corner rounding.
            // Rounded box's surface lies up to `rounding` outside the
            // raw half-extents, so the AABB grows by that radius.
            let r = prim.params[3].max(0.0);
            Vec3::new(prim.params[0], prim.params[1], prim.params[2]) + Vec3::splat(r)
        }
        TYPE_CAPSULE => {
            // Capsule oriented along local Y (matches `sdf_capsule_y`):
            // a cylinder of half-height `h` capped by hemispheres of
            // radius `r`. Bounds: `±r` in X/Z, `±(h + r)` in Y.
            let h = prim.params[0];
            let r = prim.params[1];
            Vec3::new(r, h + r, r)
        }
        TYPE_CYLINDER => {
            // Capped cylinder along local Y. Bounds: `±r` in X/Z,
            // `±h` in Y (no hemisphere caps).
            let h = prim.params[0];
            let r = prim.params[1];
            Vec3::new(r, h, r)
        }
        TYPE_TORUS => {
            // Torus in the XZ plane (matches `sdf_torus`): ring radius
            // `major`, tube radius `minor`. Bounds: `±(major + minor)`
            // in XZ, `±minor` in Y.
            let major = prim.params[0];
            let minor = prim.params[1];
            Vec3::new(major + minor, minor, major + minor)
        }
        // TYPE_PLANE is handled by the caller.
        _ => Vec3::splat(PLANE_HALF_EXTENT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    const EPS: f32 = 1e-4;

    fn prim(type_tag: u32, position: [f32; 3], rotation: Quat, scale: [f32; 3], params: [f32; 4]) -> SdfPrimitive {
        SdfPrimitive {
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
        let p = prim(TYPE_SPHERE, [0.0; 3], Quat::IDENTITY, [1.0; 3], [2.5, 0.0, 0.0, 0.0]);
        approx_aabb(primitive_aabb(&p, 0.0), Vec3::splat(-2.5), Vec3::splat(2.5));
    }

    #[test]
    fn sphere_translated_and_inflated() {
        let p = prim(TYPE_SPHERE, [10.0, -3.0, 5.0], Quat::IDENTITY, [1.0; 3], [1.0, 0.0, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.5),
            Vec3::new(8.5, -4.5, 3.5),
            Vec3::new(11.5, -1.5, 6.5),
        );
    }

    #[test]
    fn sphere_scaled_anisotropically() {
        let p = prim(TYPE_SPHERE, [0.0; 3], Quat::IDENTITY, [2.0, 1.0, 3.0], [1.0, 0.0, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-2.0, -1.0, -3.0),
            Vec3::new(2.0, 1.0, 3.0),
        );
    }

    #[test]
    fn box_with_rounding() {
        let p = prim(TYPE_BOX, [0.0; 3], Quat::IDENTITY, [1.0; 3], [1.0, 2.0, 3.0, 0.1]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-1.1, -2.1, -3.1),
            Vec3::new(1.1, 2.1, 3.1),
        );
    }

    #[test]
    fn box_rotated_45_in_y() {
        // 1×1×1 box rotated 45° around Y → AABB grows to (√2)/1 ≈ 1.414 in X/Z.
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let p = prim(TYPE_BOX, [0.0; 3], q, [1.0; 3], [1.0, 1.0, 1.0, 0.0]);
        let aabb = primitive_aabb(&p, 0.0);
        let expected = std::f32::consts::SQRT_2;
        assert!((aabb.max.x - expected).abs() < EPS);
        assert!((aabb.max.z - expected).abs() < EPS);
        // Y unchanged by Y-axis rotation.
        assert!((aabb.max.y - 1.0).abs() < EPS);
    }

    #[test]
    fn capsule_canonical() {
        // half_height = 2, radius = 0.5 → AABB ±0.5 in X/Z, ±2.5 in Y.
        let p = prim(TYPE_CAPSULE, [0.0; 3], Quat::IDENTITY, [1.0; 3], [2.0, 0.5, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-0.5, -2.5, -0.5),
            Vec3::new(0.5, 2.5, 0.5),
        );
    }

    #[test]
    fn cylinder_no_caps() {
        // Cylinder is capped flat — bounds are tight `±h` in Y.
        let p = prim(TYPE_CYLINDER, [0.0; 3], Quat::IDENTITY, [1.0; 3], [3.0, 1.0, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-1.0, -3.0, -1.0),
            Vec3::new(1.0, 3.0, 1.0),
        );
    }

    #[test]
    fn torus_xz_plane() {
        // major = 2, minor = 0.3 → AABB ±2.3 in X/Z, ±0.3 in Y.
        let p = prim(TYPE_TORUS, [0.0; 3], Quat::IDENTITY, [1.0; 3], [2.0, 0.3, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-2.3, -0.3, -2.3),
            Vec3::new(2.3, 0.3, 2.3),
        );
    }

    #[test]
    fn plane_returns_sentinel_extent() {
        let p = prim(TYPE_PLANE, [0.0, 1.0, 0.0], Quat::IDENTITY, [1.0; 3], [0.0; 4]);
        let aabb = primitive_aabb(&p, 0.0);
        assert_eq!(aabb.min, Vec3::splat(-PLANE_HALF_EXTENT) + Vec3::Y);
        assert_eq!(aabb.max, Vec3::splat(PLANE_HALF_EXTENT) + Vec3::Y);
    }

    #[test]
    fn plane_ignores_rotation_and_inflation() {
        // Rotated plane still covers everything; inflation does not
        // shrink the sentinel either.
        let q = Quat::from_rotation_x(1.234);
        let p = prim(TYPE_PLANE, [0.0; 3], q, [1.0; 3], [0.0; 4]);
        let aabb = primitive_aabb(&p, 100.0);
        assert_eq!(aabb.min, Vec3::splat(-PLANE_HALF_EXTENT));
        assert_eq!(aabb.max, Vec3::splat(PLANE_HALF_EXTENT));
    }

    #[test]
    fn inflation_negative_is_clamped_to_zero() {
        // Negative inflation must not shrink the AABB below the
        // primitive's own bounds — that would break the cull.
        let p = prim(TYPE_SPHERE, [0.0; 3], Quat::IDENTITY, [1.0; 3], [1.0, 0.0, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, -10.0),
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
        );
    }

    #[test]
    fn negative_scale_is_absolute() {
        // Mirroring via negative scale must not invert the AABB
        // (max < min would be a degenerate / inverted box).
        let p = prim(TYPE_SPHERE, [0.0; 3], Quat::IDENTITY, [-2.0, -1.0, -3.0], [1.0, 0.0, 0.0, 0.0]);
        approx_aabb(
            primitive_aabb(&p, 0.0),
            Vec3::new(-2.0, -1.0, -3.0),
            Vec3::new(2.0, 1.0, 3.0),
        );
    }
}
