//! [`PostVolumeVisualizer`] — where a post-process volume reaches full strength (#1222).
//!
//! The shape itself is the sensor's, and the collider gizmo already draws it. What this adds is the
//! inner boundary: a blend distance in from the surface, the effect is all of it. Between the two
//! wires is the fade.

use glam::{Mat3, Vec3};

use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::post_process_volume::PostProcessVolume;
use kooch_gizmos::{Gizmos, Visualizer};
use kooch_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID};

/// The inner wire's colour — the same family as the collider's green, dimmer, because it is the
/// same region seen twice and not a second object.
const FULL: Vec3 = Vec3::new(0.25, 0.65, 0.85);

#[derive(Default)]
pub(crate) struct PostVolumeVisualizer;

impl Visualizer<PostProcessVolume> for PostVolumeVisualizer {
    fn draw_with(
        &self,
        volume: &PostProcessVolume,
        transform: &GlobalTransform,
        entity: Entity,
        resources: &Resources,
        gizmos: &mut Gizmos<'_>,
    ) {
        // A global volume has no region to draw, and a volume that cuts has no fade to show.
        if volume.global || volume.blend_distance <= 0.0 {
            return;
        }
        let Some(collider) = resources
            .get::<kooch_ecs::component::ComponentRegistry>()
            .and_then(|registry| registry.get_cpu::<Collider>()?.get(entity).copied())
        else {
            return;
        };
        let (scale, rotation, translation) = transform.matrix.to_scale_rotation_translation();
        let basis = Mat3::from_quat(rotation);
        let s = scale.abs();
        let centre = translation + rotation * (collider.center * s);
        // In world units, like the blend distance itself: an inset in local space would shrink with
        // the region rather than staying the metre the author typed.
        let inset = volume.blend_distance;
        match collider.shape {
            SHAPE_CUBOID => {
                let half = (collider.half_extents * s - Vec3::splat(inset)).max(Vec3::ZERO);
                gizmos.wire_obb(centre, basis, half, FULL);
            }
            SHAPE_CAPSULE => {
                let radius = (collider.radius * s.x.max(s.z) - inset).max(0.0);
                gizmos.wire_capsule(centre, basis, radius, collider.half_height * s.y, FULL);
            }
            // Sphere for the sphere, and for the shapes with no inner wire worth guessing at: a
            // hull inset by a metre is not a hull scaled by anything.
            _ => {
                let radius = (collider.radius * s.max_element() - inset).max(0.0);
                gizmos.wire_sphere(centre, basis, radius, FULL);
            }
        }
    }

    fn draw(&self, _volume: &PostProcessVolume, _transform: &GlobalTransform, _: &mut Gizmos<'_>) {
        // The region lives on the collider, which `draw_with` is the only one that can reach.
    }
}
