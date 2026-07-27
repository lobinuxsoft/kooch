//! Resources-side input application: runs **outside** the egui closure
//! and turns a captured [`ViewportInputDelta`] into mutations on the
//! editor camera entity's `Transform` plus the
//! [`EditorCameraController`] resource, then propagates `GlobalTransform`
//! so the renderer sees the new pose this same frame.

use std::any::TypeId;

use glam::{Quat, Vec3};
use ome_core::resource::Resources;
use ome_core::time::Time;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use ome_ecs::transform::Transform;

use crate::editor_camera::controller::EditorCameraController;
use crate::editor_camera::fly::fly_velocity;
use crate::editor_camera::markers::EditorCamera;
use crate::editor_camera::orbit::{apply_yaw_pitch, camera_position, fly_look_pivot_camera};
use crate::editor_camera::pan_zoom::{apply_zoom, pan_delta};

use super::ViewportInputDelta;

/// Applies a captured input delta to the editor camera entity.
///
/// Mutates the controller's focus point / distance, the camera
/// `Transform`, and triggers a hierarchy propagation so the renderer
/// reads the updated `GlobalTransform` on this same frame.
///
/// `selection_world_position` is the world-space position used to
/// re-centre the camera when the user pressed `F`. `None` means "no
/// selection" — focus-on-selection is a no-op in that case.
pub fn apply_viewport_input(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selection_world_position: Option<Vec3>,
) {
    if delta.is_idle() {
        return;
    }

    let Some(entity) = find_editor_camera_entity(resources) else {
        return;
    };

    let dt = resources
        .get::<Time>()
        .map(|t| t.delta_secs())
        .unwrap_or(1.0 / 60.0);

    // --- Snapshot controller and current transform ------------------------
    let mut controller = match resources.get::<EditorCameraController>() {
        Some(c) => c.clone(),
        None => return,
    };

    // Position is purely derived from focus_point/orientation/distance, so
    // we ignore the existing position and recompute it after mutations.
    let Some((_, mut rotation)) = read_transform(resources, entity) else {
        return;
    };

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
        controller.focus_point += world_delta;
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
    //
    // FPS look pivots around the *camera*, not around `focus_point`.
    // `fly_look_pivot_camera` rotates and re-anchors `focus_point` so
    // the derived camera position stays fixed under pure rotation.
    // WASD/QE then translates camera and focus together so the in-front
    // pivot moves with the camera.
    if delta.fly_active {
        if delta.fly_yaw != 0.0 || delta.fly_pitch != 0.0 {
            let position_before =
                camera_position(controller.focus_point, rotation, controller.distance);
            let (new_rotation, new_focus) = fly_look_pivot_camera(
                position_before,
                rotation,
                controller.distance,
                delta.fly_yaw,
                delta.fly_pitch,
            );
            rotation = new_rotation;
            controller.focus_point = new_focus;
        }

        let velocity = fly_velocity(delta.fly_keys, rotation, controller.fly_speed, dt);
        if velocity != Vec3::ZERO {
            controller.focus_point += velocity;
        }
    }

    // --- Focus on selection (F) -------------------------------------------
    if delta.focus_pressed
        && let Some(target) = selection_world_position
    {
        controller.focus_point = target;
    }

    // --- Recompute position from the (possibly updated) state -------------
    let position = camera_position(controller.focus_point, rotation, controller.distance);

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

fn find_editor_camera_entity(resources: &Resources) -> Option<Entity> {
    let archetypes = resources.get::<ArchetypeRegistry>()?;
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    for arch in archetypes.iter_matching(&[]) {
        if arch.components().contains(&editor_camera_tid) {
            return arch.entities().first().copied();
        }
    }
    None
}

fn read_transform(resources: &Resources, entity: Entity) -> Option<(Vec3, Quat)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<Transform>()?;
    let t = storage.get(entity)?;
    Some((t.position, t.rotation))
}

fn write_transform(resources: &mut Resources, entity: Entity, position: Vec3, rotation: Quat) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
        && let Some(t) = storage.get_mut(entity)
    {
        t.position = position;
        t.rotation = rotation;
    }
}
