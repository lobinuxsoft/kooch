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
use kooch_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID, SHAPE_SPHERE};

/// The inner wire's colour — the same family as the collider's green, dimmer, because it is the
/// same region seen twice and not a second object.
const FULL: Vec3 = Vec3::new(0.25, 0.65, 0.85);

/// The same wire while something is inside it. A region that looks identical whether or not it is
/// firing is a region an author debugs by guessing.
const OCCUPIED: Vec3 = Vec3::new(1.0, 0.65, 0.2);

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
        let Some(registry) = resources.get::<kooch_ecs::component::ComponentRegistry>() else {
            return;
        };
        let colour = match resources
            .get::<kooch_ecs::sensor_occupancy::SensorOccupancy>()
            .and_then(|inside| inside.depth_in(entity))
        {
            Some(depth) if depth >= 0.0 => OCCUPIED,
            _ => FULL,
        };
        // Every collider under the region, not one: the solver builds a compound out of a parent
        // and its descendants, so an author covers an awkward space with several shapes and has to
        // see all of them (#1222).
        let mut stack = vec![entity];
        while let Some(part) = stack.pop() {
            if let Some(children) = registry
                .get_cpu::<kooch_ecs::hierarchy::Children>()
                .and_then(|storage| storage.get(part))
            {
                stack.extend(children.entities.iter().copied());
            }
            let (Some(collider), Some(at)) = (
                registry.get_cpu::<Collider>().and_then(|s| s.get(part)),
                match part == entity {
                    true => Some(transform),
                    false => registry
                        .get_cpu::<GlobalTransform>()
                        .and_then(|s| s.get(part)),
                },
            ) else {
                continue;
            };
            draw_inner(collider, at, volume.blend_distance, colour, gizmos);
        }
    }

    fn draw(&self, _volume: &PostProcessVolume, _transform: &GlobalTransform, _: &mut Gizmos<'_>) {
        // The region lives on the colliders, which `draw_with` is the only one that can reach.
    }
}

/// The wire a blend distance inside one collider's surface.
fn draw_inner(
    collider: &Collider,
    transform: &GlobalTransform,
    inset: f32,
    colour: Vec3,
    gizmos: &mut Gizmos<'_>,
) {
    let (scale, rotation, translation) = transform.matrix.to_scale_rotation_translation();
    let basis = Mat3::from_quat(rotation);
    let s = scale.abs();
    let centre = translation + rotation * (collider.center * s);
    // In world units, like the blend distance itself: an inset in local space would shrink with
    // the region rather than staying the metre the author typed.
    match collider.shape {
        SHAPE_CUBOID => {
            let half = (collider.half_extents * s - Vec3::splat(inset)).max(Vec3::ZERO);
            gizmos.wire_obb(centre, basis, half, colour);
        }
        SHAPE_CAPSULE => {
            let radius = (collider.radius * s.x.max(s.z) - inset).max(0.0);
            gizmos.wire_capsule(centre, basis, radius, collider.half_height * s.y, colour);
        }
        // A mesh-derived shape has no inner wire worth guessing at: a hull inset by a metre is
        // not a hull scaled by anything, and a wrong one would be read as the truth.
        SHAPE_SPHERE => {
            let radius = (collider.radius * s.max_element() - inset).max(0.0);
            gizmos.wire_sphere(centre, basis, radius, colour);
        }
        _ => {}
    }
}
