//! One camera, as a value a pass can be handed.

use glam::{Mat4, Vec3};

use kooch_ecs::PerspectiveCamera;
use kooch_ecs::hierarchy::GlobalTransform;

/// The camera a single view is rendered through.
#[derive(Debug, Clone, Copy)]
pub struct ViewCamera {
    /// Camera-to-world. Its translation is the eye position.
    pub world_matrix: Mat4,
    pub fov_y_rad: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for ViewCamera {
    /// A plausible lens at the origin, looking down -Z.
    fn default() -> Self {
        Self {
            world_matrix: Mat4::IDENTITY,
            fov_y_rad: std::f32::consts::FRAC_PI_3,
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl ViewCamera {
    /// Reads one from the components an entity carries, clamping the lens
    /// to values a projection matrix survives.
    pub fn from_components(cam: &PerspectiveCamera, transform: &GlobalTransform) -> Self {
        Self {
            world_matrix: transform.matrix,
            fov_y_rad: cam.fov.to_radians().max(1.0_f32.to_radians()),
            near: cam.near.max(0.001),
            far: cam.far.max(cam.near + 0.001),
        }
    }

    /// A camera at `eye` pointed at `target`, with the default lens.
    pub fn looking_at(eye: Vec3, target: Vec3) -> Self {
        let direction = (target - eye).normalize_or(Vec3::NEG_Z);
        let up = if direction.y.abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        Self {
            world_matrix: Mat4::look_at_rh(eye, target, up).inverse(),
            ..Default::default()
        }
    }

    /// World-to-camera.
    pub fn view(&self) -> Mat4 {
        self.world_matrix.inverse()
    }

    /// Reverse-Z perspective for a target of this aspect ratio.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        crate::projection::perspective_infinite_rh_reverse_z(
            self.fov_y_rad,
            aspect.max(0.01),
            self.near,
        )
    }

    /// A **bounded** reverse-Z projection, cut short at `far`.
    pub fn projection_to(&self, aspect: f32, far: f32) -> Mat4 {
        crate::projection::perspective_rh_reverse_z(
            self.fov_y_rad,
            aspect.max(0.01),
            self.near,
            far.max(self.near + 1e-3),
        )
    }

    /// Unit vector down the view axis, in world space.
    pub fn forward(&self) -> Vec3 {
        self.world_matrix
            .transform_vector3(Vec3::NEG_Z)
            .normalize_or(Vec3::NEG_Z)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Eye position in world space.
    pub fn position(&self) -> Vec3 {
        self.world_matrix.w_axis.truncate()
    }
}
