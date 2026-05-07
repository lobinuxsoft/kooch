//! Reversed-Z projection helpers (#488).
//!
//! Standard wgpu/D3D depth maps near→0, far→1, which clusters
//! IEEE-754 float precision near the FAR plane (where it's wasted on
//! geometry the camera barely sees) and starves the near plane of
//! resolution. **Reversed-Z** flips the orientation: near→1, far→0.
//! Combined with a `Greater` depth comparison, the resulting depth
//! distribution puts most of the float precision exactly where the
//! eye actually looks.
//!
//! Modern engines (UE5, Unity HDRP, Bevy) all use reversed-Z. The
//! Hi-Z occlusion cull port from Bevy's
//! [`meshlet_cull_shared.wgsl`](https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/meshlet/meshlet_cull_shared.wgsl)
//! is correctness-tied to this orientation — both the comparison
//! operator (`<=` vs `>=`) and the pyramid reduce direction (`min`
//! vs `max`) flip with it.
//!
//! Migration checklist (callers MUST do all four):
//! 1. Replace `Mat4::perspective_rh` with [`perspective_rh_reverse_z`].
//! 2. Flip `wgpu::CompareFunction::Less` → `Greater` (and `LessEqual`
//!    → `GreaterEqual`) in every render pipeline that consumes depth.
//! 3. Flip depth attachment `LoadOp::Clear(1.0)` → `Clear(0.0)`.
//! 4. Flip Hi-Z pyramid reduce: `max` → `min` (in `hi_z_spd.wgsl`).

use glam::{Mat4, Vec4};

/// Right-handed perspective projection with **reversed-Z** depth: the
/// near plane maps to `ndc.z = 1.0` and the far plane to `ndc.z = 0.0`.
///
/// Drop-in replacement for [`glam::Mat4::perspective_rh`] for any
/// camera that participates in the depth pipeline. See module docs
/// for the rest of the migration steps.
///
/// # Implementation
///
/// Builds the standard `perspective_rh` (which produces depth `[0, 1]`
/// near→far) and pre-multiplies by a depth-flip matrix that maps
/// `ndc.z'` to `1 - ndc.z`. After the perspective divide:
///
/// ```text
/// clip.z' = -clip.z + clip.w
/// ndc.z'  = clip.z' / clip.w = 1 - ndc.z
/// ```
///
/// For finite `near`/`far` this is numerically equivalent to
/// constructing the reversed-Z projection coefficients directly; we
/// keep the multiplicative form because it's easier to reason about
/// and the MAD cost is irrelevant on a per-frame matrix build.
pub fn perspective_rh_reverse_z(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let depth_flip = Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );
    depth_flip * Mat4::perspective_rh(fovy, aspect, near, far)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4Swizzles};

    fn project_point(proj: Mat4, view_z: f32) -> f32 {
        let clip = proj * Vec4::new(0.0, 0.0, view_z, 1.0);
        clip.z / clip.w
    }

    #[test]
    fn near_plane_maps_to_one() {
        let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        // RH cameras look down -Z; the near plane is at view_z = -near.
        let z = project_point(proj, -0.1);
        assert!((z - 1.0).abs() < 1e-3, "near plane should map to ndc.z ≈ 1.0, got {z}");
    }

    #[test]
    fn far_plane_maps_to_zero() {
        let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let z = project_point(proj, -100.0);
        assert!(z.abs() < 1e-3, "far plane should map to ndc.z ≈ 0.0, got {z}");
    }

    #[test]
    fn midpoint_lies_between() {
        // Reversed-Z spreads precision NON-uniformly in view space —
        // points closer to the camera get more depth resolution. The
        // mid-distance point lands somewhere between 0 and 1, NOT
        // exactly 0.5, but ordering is preserved monotonically with
        // distance.
        let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let z_close = project_point(proj, -1.0);
        let z_mid = project_point(proj, -50.0);
        let z_far = project_point(proj, -100.0);
        assert!(z_close > z_mid, "closer point must have larger ndc.z (reversed-Z)");
        assert!(z_mid > z_far, "mid point must have larger ndc.z than far");
        assert!((0.0..=1.0).contains(&z_mid), "ndc.z must stay in [0, 1]");
    }

    #[test]
    fn xy_unchanged_versus_standard() {
        // The depth flip only touches z; xy must match standard perspective.
        let std = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let rev = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let p = Vec4::new(1.0, 0.5, -10.0, 1.0);
        let s = std * p;
        let r = rev * p;
        assert_eq!(s.x, r.x);
        assert_eq!(s.y, r.y);
        assert_eq!(s.w, r.w);
    }

    #[test]
    fn world_corner_round_trip() {
        // Sanity: a world-space point at the centre of the frustum
        // projects somewhere visible in NDC.
        let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let view_proj = proj * view;
        let p = view_proj * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let ndc = p.xyz() / p.w;
        assert!(ndc.x.abs() < 0.5);
        assert!(ndc.y.abs() < 0.5);
        // Origin is between near (5 - 0.1) and far (5 + 100), should
        // give a smallish ndc.z (closer to 0 than to 1 — well into the
        // reversed-Z far band).
        assert!((0.0..=1.0).contains(&ndc.z));
    }
}
