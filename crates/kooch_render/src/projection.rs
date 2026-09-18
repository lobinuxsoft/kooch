//! Reversed-Z projection helpers (#488).

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
    depth_flip * glam::camera::rh::proj::directx::perspective(fovy, aspect, near, far)
}

/// Right-handed **reversed-Z with no far plane**: near maps to `ndc.z = 1.0` and infinity to `ndc.z
/// = 0.0`, which it approaches without reaching.
pub fn perspective_infinite_rh_reverse_z(fovy: f32, aspect: f32, near: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective_infinite_reverse(fovy, aspect, near)
}

/// A world-space ray: where a screen pixel points once it leaves the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldRay {
    pub origin: Vec3,
    /// Normalised, pointing away from the camera.
    pub direction: Vec3,
}

/// Unprojects a viewport-local cursor position into a world-space ray.
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

    // 🔴 Unprojected at the NEAR plane (`ndc.z = 1` under reversed-Z), not the far one. There is no
    // far plane any more: `ndc.z = 0` is infinity, it unprojects to `w = 0`, and every pick would
    // return `None`.
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
    pub fn at(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

#[cfg(test)]
mod tests;
