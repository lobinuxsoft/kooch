//! One camera, as a value a pass can be handed.
//!
//! # Why
//!
//! Four places used to answer "which camera is this frame drawn from" by
//! querying the world for the highest-priority active `PerspectiveCamera`:
//! the game runtime, the editor viewport, the gizmo ray builder and the
//! sky pass. That works while there is exactly one view. With two, the
//! sky pass answered the world's question instead of its caller's, so
//! orbiting the editor camera swung the sky in the Game panel while the
//! meshlet stage — which had already learned to take a view — held still.
//!
//! A pass that resolves its own camera cannot be asked to draw a second
//! view. So the camera becomes a parameter: the caller knows which view
//! it is rendering, and it is the only one that can know.

use glam::{Mat4, Vec3};

use kooch_ecs::PerspectiveCamera;
use kooch_ecs::hierarchy::GlobalTransform;

/// The camera a single view is rendered through.
///
/// Holds the world transform and the lens, not the derived matrices:
/// aspect ratio belongs to the target being drawn into, and two panels
/// showing the same camera at different sizes need different projections
/// from the same `ViewCamera`.
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
    ///
    /// What a view renders through when the scene has no camera at all —
    /// the frame after a project opens, or a game that has not spawned
    /// one. The alternative callers used was an identity matrix, which
    /// is not a projection: it makes clip space the unit cube in view
    /// space, so the near plane is behind the eye and the shadow
    /// cascades that read the near and far planes get 1 and -1.
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
    ///
    /// Stores the same camera-to-world an entity would carry, so a
    /// caller that has a place to look from rather than a transform does
    /// not have to remember which way round the inverse goes. Falls back
    /// to a Z up vector when the view direction is vertical, where a Y
    /// up is degenerate and `look_at_rh` returns NaN.
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
        self.projection_to(aspect, self.far)
    }

    /// The same projection, cut short at `far`.
    ///
    /// Shadow cascades are placed against a frustum that stops at the
    /// shadow distance rather than at the camera's far plane, which on a
    /// planet is kilometres away. Fitting four cascades to *that* puts
    /// the near one hundreds of metres deep and every shadow in the
    /// scene turns to mush — the split scheme is doing exactly what it
    /// was asked, against the wrong range.
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
