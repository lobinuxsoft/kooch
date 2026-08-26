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

use glam::{Mat4, Vec2, Vec3, Vec4};

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

/// Right-handed **reversed-Z with no far plane**: near maps to
/// `ndc.z = 1.0` and infinity to `ndc.z = 0.0`, which it approaches
/// without reaching.
///
/// This is what a camera in this engine renders with. The finite form
/// above survives for the one job that genuinely needs a bounded
/// frustum — fitting shadow cascades to a slice of it.
///
/// # Why no far plane
///
/// Two reasons, and the second is the one that decided it.
///
/// **Precision.** Reversed-Z spends float exponent where the eye is,
/// and a finite far plane spends some of that range describing the
/// distance between `far` and infinity, which nothing renders.
///
/// **`ndc.z` becomes a usable number.** With this projection
/// `ndc.z = near / distance` exactly, so any shader can recover metres
/// with one divide and no extra uniform. With a finite far it is
/// `A / (B − ndc.z)`, which needs two coefficients plumbed to every
/// consumer — and every one that forgets is a technique whose
/// world-space parameters quietly mean something different per scene.
/// Screen-space contact shadows (#735) hit that first; SSR, SSAO, fog,
/// temporal upscaling (#732) and the atmosphere (#248) all read depth
/// the same way and would each have hit it in turn.
///
/// It is also what Bevy does — `bevy_camera/src/projection.rs`, whose
/// `Perspective` has a `far` field that its own `get_clip_from_view`
/// does not pass to `Mat4::perspective_infinite_reverse_rh`. Every
/// depth helper they ship (`depth_ndc_to_view_z`, the SSR/contact-shadow
/// ray march) assumes it, so porting their shaders and not their
/// projection is porting half a mechanism.
pub fn perspective_infinite_rh_reverse_z(fovy: f32, aspect: f32, near: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fovy, aspect, near)
}

/// The `near` a shader can recover from the projection matrix alone,
/// which is all a depth linearisation needs once the far plane is gone.
///
/// Bevy reads exactly this element and calls it
/// `perspective_camera_near()` (`view_transformations.wgsl:166`); the
/// name here says where it comes from, because on any other projection
/// the element means something else.
pub fn near_from_infinite_projection(proj: Mat4) -> f32 {
    proj.to_cols_array_2d()[3][2]
}

/// A world-space ray: where a screen pixel points once it leaves the
/// camera.
///
/// Not [`kooch_gizmos_handles::Ray`] — that crate deliberately depends on
/// nothing but `glam`, and pulling it in here to share a two-field struct
/// would buy a type at the cost of the isolation that makes it testable.
/// This follows the convention `kooch_core::aabb::ray_intersect` already
/// uses: geometry travels as an origin and a direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldRay {
    pub origin: Vec3,
    /// Normalised, pointing away from the camera.
    pub direction: Vec3,
}

/// Unprojects a viewport-local cursor position into a world-space ray.
///
/// The inverse of [`perspective_rh_reverse_z`], and it lives beside it for
/// that reason: **the far plane is `ndc.z = 0`, not 1** (#488). Unprojecting
/// with the conventional orientation yields a ray pointing the wrong way
/// down the frustum, and a caller who reads a wrong-but-plausible position
/// out of it has nothing to tell them why. Anything that changes the
/// convention has to change this in the same edit.
///
/// `cursor` and `viewport_size` are in egui's coordinates — pixels from the
/// viewport's top-left, **Y down**. NDC's Y is up, hence the flip below.
///
/// `camera_to_world` is the camera's `GlobalTransform` matrix, not a view
/// matrix; the view matrix is its inverse.
///
/// Returns `None` for a degenerate viewport or a camera whose matrix cannot
/// be inverted, rather than a ray built from a division by zero.
///
/// # Uses
///
/// Gizmo handle picking and dragging, dropping an asset into the viewport
/// at the cursor, and entity picking when that arrives.
pub fn viewport_cursor_to_ray(
    cursor: Vec2,
    viewport_size: Vec2,
    camera_to_world: Mat4,
    fov_y_radians: f32,
    near: f32,
) -> Option<WorldRay> {
    if viewport_size.x < 1.0 || viewport_size.y < 1.0 {
        return None;
    }
    let aspect = (viewport_size.x / viewport_size.y).max(0.001);
    let near = near.max(0.001);
    let proj = perspective_infinite_rh_reverse_z(fov_y_radians, aspect, near);
    let view_proj = proj * camera_to_world.inverse();
    let inverse = view_proj.inverse();

    // Cursor to NDC. egui measures Y downwards from the top; NDC measures
    // it upwards from the centre.
    let ndc_x = 2.0 * (cursor.x / viewport_size.x) - 1.0;
    let ndc_y = 1.0 - 2.0 * (cursor.y / viewport_size.y);

    // 🔴 Unprojected at the NEAR plane (`ndc.z = 1` under reversed-Z),
    // not the far one. There is no far plane any more: `ndc.z = 0` is
    // infinity, it unprojects to `w = 0`, and every pick would return
    // `None`. The near point is on the same ray through the eye and is
    // the only one that stays finite.
    let near_point = inverse * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    if near_point.w.abs() < 1e-6 {
        return None;
    }
    let near_point = near_point.truncate() / near_point.w;
    let origin = camera_to_world.w_axis.truncate();
    let direction = (near_point - origin).normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    Some(WorldRay { origin, direction })
}

impl WorldRay {
    /// Where this ray crosses the horizontal plane at `height`.
    ///
    /// `None` when the ray runs parallel to the plane, or crosses it
    /// *behind* the camera — both cases where a caller placing something at
    /// the returned point would put it somewhere the user did not click.
    /// Looking at the horizon is the ordinary way to reach them, so this is
    /// a case to handle rather than an error.
    pub fn hits_horizontal_plane(&self, height: f32) -> Option<Vec3> {
        if self.direction.y.abs() < 1e-6 {
            return None;
        }
        let distance = (height - self.origin.y) / self.direction.y;
        match distance > 0.0 {
            true => Some(self.origin + self.direction * distance),
            false => None,
        }
    }

    /// The point `distance` along the ray.
    ///
    /// The fallback when [`Self::hits_horizontal_plane`] finds nothing:
    /// something in front of the camera beats refusing to place anything.
    pub fn at(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

#[cfg(test)]
mod tests;
