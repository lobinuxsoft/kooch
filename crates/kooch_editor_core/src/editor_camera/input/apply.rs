//! Resources-side input application.

use glam::{DVec3, Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use kooch_ecs::transform::Transform;

use crate::editor_camera::controller::EditorCameraController;
use crate::editor_camera::find_editor_camera_entity;
use crate::editor_camera::fly::fly_velocity;
use crate::editor_camera::orbit::{apply_yaw_pitch, fly_look_pivot_camera};
use crate::editor_camera::pan_zoom::{apply_zoom, pan_delta};

use super::ViewportInputDelta;

/// Applies a captured input delta to the editor camera entity.
pub fn apply_viewport_input(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    focus_target: Option<crate::editor_camera::framing::FocusTarget>,
) {
    if delta.is_idle() {
        return;
    }

    let Some(entity) = find_editor_camera_entity(resources) else {
        return;
    };

    let dt = Time::of(resources).unscaled_delta_secs();

    // --- Snapshot controller and current transform ------------------------
    let mut controller = match resources.get::<EditorCameraController>() {
        Some(c) => c.clone(),
        None => return,
    };

    // Position is purely derived from focus_point/orientation/distance, so
    // we ignore the existing position and recompute it after mutations.
    let Some(mut rotation) = read_rotation(resources, entity) else {
        return;
    };
    let origin = kooch_ecs::hierarchy::WorldOrigin::of(resources);
    controller.max_distance = reach(resources, controller.focus_point);

    // --- Axis-gizmo snap --------------------------------------------------- Before the orbit so a
    // click and a stray drag in the same frame resolve to the clicked view. Focus point and
    // distance survive; only the orientation jumps, which is what the gizmo promises.
    if let Some(snap) = delta.snap_orientation {
        rotation = snap;
    }

    // --- Orbit (MMB drag, no Shift) ---------------------------------------
    if delta.orbit_yaw != 0.0 || delta.orbit_pitch != 0.0 {
        rotation = apply_yaw_pitch(rotation, delta.orbit_yaw, delta.orbit_pitch);
    }

    // --- Pan (Shift + MMB drag) -------------------------------------------
    if delta.pan_dx != 0.0 || delta.pan_dy != 0.0 {
        let world_delta = pan_delta(
            delta.pan_dx,
            delta.pan_dy,
            controller.effective_pan_sensitivity(),
            rotation,
        );
        controller.focus_point += world_delta.as_dvec3();
    }

    // --- Zoom (mouse wheel) -----------------------------------------------
    if delta.zoom_lines != 0.0 {
        controller.distance = apply_zoom(
            controller.distance,
            delta.zoom_lines,
            controller.zoom_sensitivity,
        );
        controller.clamp_distance();
    }

    // --- Fly-mode look + WASD/QE ------------------------------------------
    if delta.fly_active {
        if delta.fly_yaw != 0.0 || delta.fly_pitch != 0.0 {
            let position_before = eye(&controller, rotation);
            // Pivoted at zero and added in f64: the eye can be a planet radius out.
            let (new_rotation, to_focus) = fly_look_pivot_camera(
                Vec3::ZERO,
                rotation,
                controller.distance,
                delta.fly_yaw,
                delta.fly_pitch,
            );
            rotation = new_rotation;
            controller.focus_point = position_before + to_focus.as_dvec3();
        }

        // Faster the further out: 5 m/s at a prop, a planet crossed in seconds from orbit (#1970).
        let speed = controller.fly_speed.max(controller.distance * 0.5);
        let velocity = fly_velocity(delta.fly_keys, rotation, speed, dt);
        if velocity != Vec3::ZERO {
            controller.focus_point += velocity.as_dvec3();
        }
    }

    // --- Focus on selection (F) -------------------------------------------
    if delta.focus_pressed
        && let Some(target) = focus_target
    {
        controller.focus_point = origin + target.point.as_dvec3();
        // Centring alone left a block a hundred metres away in the
        // middle of the screen. The distance is what makes F *frame*
        // rather than merely aim.
        controller.distance =
            crate::editor_camera::framing::distance_for(target.radius, camera_fov(resources));
        controller.max_distance = controller.max_distance.max(controller.distance);
        controller.clamp_distance();
    }

    // --- Recompute position from the (possibly updated) state -------------
    let position = eye(&controller, rotation);
    write_transform(resources, entity, position, rotation);
    if let Some(c) = resources.get_mut::<EditorCameraController>() {
        *c = controller;
    }

    // Propagate GlobalTransform so the same-frame renderer sees the
    // updated world matrix without waiting for PostUpdate.
    transform_propagation_system(resources);
}

/// Returns the world-space position of `entity` from its `GlobalTransform`,
/// for focus-on-selection. `None` when the entity has no `GlobalTransform`.
pub fn entity_world_position(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<GlobalTransform>()?;
    let gt = storage.get(entity)?;
    let (_, _, translation) = gt.matrix.to_scale_rotation_translation();
    Some(translation)
}

fn read_rotation(resources: &Resources, entity: Entity) -> Option<Quat> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<Transform>()?;
    Some(storage.get(entity)?.rotation)
}

/// The eye, in absolute f64: `distance` back from the focus along the view.
fn eye(controller: &EditorCameraController, rotation: Quat) -> DVec3 {
    let forward = (rotation * -Vec3::Z).as_dvec3();
    controller.focus_point - forward * f64::from(controller.distance)
}

/// How far the camera may back off: 10 km, or far enough to see every planet whole.
fn reach(resources: &Resources, focus: DVec3) -> f32 {
    const FLOOR: f64 = 10_000.0;
    let origin = kooch_ecs::hierarchy::WorldOrigin::of(resources);
    let mut far = FLOOR;
    kooch_ecs::query::Query::<(&kooch_planet::Planet, &GlobalTransform)>::new(resources).for_each(
        |(planet, global)| {
            let centre = origin + global.matrix.w_axis.truncate().as_dvec3();
            let outer = f64::from(planet.radius + planet.relief);
            far = far.max((centre - focus).length() + outer * 4.0);
        },
    );
    far.min(f64::from(f32::MAX)) as f32
}

fn write_transform(resources: &mut Resources, entity: Entity, position: DVec3, rotation: Quat) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(mut storage) = registry.get_cpu_mut::<Transform>()
        && let Some(t) = storage.get_mut(entity)
    {
        t.position = position;
        t.rotation = rotation;
    }
}

/// The active camera's vertical field of view, in radians.
fn camera_fov(resources: &Resources) -> f32 {
    crate::gizmos::active_camera(resources)
        .map(|(camera, _)| camera.fov.to_radians())
        .unwrap_or(std::f32::consts::FRAC_PI_3)
}
