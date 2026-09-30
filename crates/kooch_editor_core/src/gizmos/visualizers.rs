//! Built-in [`Visualizer`] implementations for the editor's overlay gizmos: cameras (perspective +
//! orthographic) and directional lights. Registered by
//! [`super::register_builtin_visualizers_system`].

use glam::Vec3;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::orthographic_camera::OrthographicCamera;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_gizmos::{Gizmos, Visualizer};

const FRUSTUM_COLOR: Vec3 = Vec3::new(0.4, 0.8, 1.0);
const ORTHO_COLOR: Vec3 = Vec3::new(0.6, 0.85, 1.0);
/// What a vcam asks for, in the rig's warm hue rather than the camera's blue: this is the rig's
/// doing, not the camera's own shape.
const ASKED_COLOR: Vec3 = Vec3::new(1.0, 0.85, 0.45);

/// Aspect ratio used to draw camera frustums. The viewport's actual aspect is not exposed to
/// visualizers in v1 — a fixed 16:9 keeps the frustum shape readable. Future work: read the live
/// aspect from the editor's `ViewportTarget`.
const FRUSTUM_ASPECT: f32 = 16.0 / 9.0;

/// Built-in visualizer for `PerspectiveCamera`: pyramid frustum from
/// camera origin to the far plane plus rectangles at near and far.
#[derive(Default)]
pub(crate) struct PerspectiveCameraVisualizer;

impl Visualizer<PerspectiveCamera> for PerspectiveCameraVisualizer {
    fn draw(
        &self,
        camera: &PerspectiveCamera,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        frustum(
            gizmos,
            transform.matrix,
            camera.fov,
            camera.near,
            camera.far,
            FRUSTUM_COLOR,
        );
    }
}

/// The pyramid a lens cuts out of the world: rectangles at the near and far planes and the four
/// edges between them, drawn in the space `matrix` puts them.
fn frustum(
    gizmos: &mut Gizmos<'_>,
    matrix: glam::Mat4,
    fov: f32,
    near: f32,
    far: f32,
    colour: Vec3,
) {
    let half_fov = (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan();
    // Camera looks down -Z (right-handed).
    let plane = |distance: f32| {
        let h = distance * half_fov;
        let w = h * FRUSTUM_ASPECT;
        [
            matrix.transform_point3(Vec3::new(w, h, -distance)),
            matrix.transform_point3(Vec3::new(-w, h, -distance)),
            matrix.transform_point3(Vec3::new(-w, -h, -distance)),
            matrix.transform_point3(Vec3::new(w, -h, -distance)),
        ]
    };
    let (near, far) = (plane(near), plane(far));
    for i in 0..4 {
        gizmos.line(near[i], near[(i + 1) % 4], colour);
        gizmos.line(far[i], far[(i + 1) % 4], colour);
        gizmos.line(near[i], far[i], colour);
    }
}

/// The frustum a vcam ASKS for, while it is selected (#1254).
///
/// 🔴 Drawn from the vcam's own pose, in its own colour, because it is not the camera's: a vcam
/// that asks for 30° next to one that asks for 90° is two different pictures of the same scene,
/// and the number in the Inspector does not say how different. The roll is in here too — it is the
/// only place `dutch` can be seen without pressing Play.
#[derive(Default)]
pub(crate) struct LensOverrideVisualizer;

impl Visualizer<kooch_camera::LensOverride> for LensOverrideVisualizer {
    fn draw(
        &self,
        lens: &kooch_camera::LensOverride,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let rolled = transform.matrix * glam::Mat4::from_rotation_z(-lens.dutch.to_radians());
        frustum(gizmos, rolled, lens.fov, lens.near, lens.far, ASKED_COLOR);
    }
}

/// Built-in visualizer for `OrthographicCamera`: 12-edge wireframe box
/// of the orthographic volume.
#[derive(Default)]
pub(crate) struct OrthographicCameraVisualizer;

impl Visualizer<OrthographicCamera> for OrthographicCameraVisualizer {
    fn draw(
        &self,
        camera: &OrthographicCamera,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let half_w = camera.size * FRUSTUM_ASPECT;
        let half_h = camera.size;

        // 8 corners in local space (camera looks -Z).
        let corners_local = [
            Vec3::new(half_w, half_h, -camera.near),
            Vec3::new(-half_w, half_h, -camera.near),
            Vec3::new(-half_w, -half_h, -camera.near),
            Vec3::new(half_w, -half_h, -camera.near),
            Vec3::new(half_w, half_h, -camera.far),
            Vec3::new(-half_w, half_h, -camera.far),
            Vec3::new(-half_w, -half_h, -camera.far),
            Vec3::new(half_w, -half_h, -camera.far),
        ];

        let c: [Vec3; 8] =
            std::array::from_fn(|i| transform.matrix.transform_point3(corners_local[i]));

        // Near rect, far rect, and 4 side edges.
        for i in 0..4 {
            gizmos.line(c[i], c[(i + 1) % 4], ORTHO_COLOR);
            gizmos.line(c[4 + i], c[4 + (i + 1) % 4], ORTHO_COLOR);
            gizmos.line(c[i], c[4 + i], ORTHO_COLOR);
        }
    }
}
