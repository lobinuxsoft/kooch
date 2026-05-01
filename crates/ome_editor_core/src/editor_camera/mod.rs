//! Editor camera — independent viewport navigation.
//!
//! Spawns one ECS entity tagged with [`EditorCamera`] + [`EditorOnly`] +
//! `PerspectiveCamera` + `Transform` at editor startup. The `EditorOnly`
//! marker is registered in
//! [`EphemeralComponents`](ome_ecs::EphemeralComponents) so the entity is
//! excluded from scene save and preserved across scene loads.
//!
//! Navigation state (orbit pivot, distance, sensitivities) lives in the
//! [`EditorCameraController`] resource — see its docs for the rationale.
//!
//! Input handling and the orbit/pan/zoom/fly math live in sibling
//! modules added in subsequent commits.

pub mod controller;
pub mod fly;
pub mod input;
pub mod markers;
pub mod orbit;
pub mod pan_zoom;

use std::any::TypeId;

use glam::{Mat4, Vec3};

use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::EphemeralComponents;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::transform::Transform;
use ome_world::focus::StreamingFocus;

use crate::play_state::PlayState;

pub use controller::EditorCameraController;
pub use markers::{EditorCamera, EditorOnly};

/// Priority assigned to the editor camera's `PerspectiveCamera`. Chosen
/// well above any plausible gameplay camera priority so the editor view
/// always wins while the marker is `active`.
pub const EDITOR_CAMERA_PRIORITY: i32 = 1000;

/// Default world-space spawn position for the editor camera.
const DEFAULT_EYE: Vec3 = Vec3::new(5.0, 5.0, 5.0);

/// Startup system: registers `EditorOnly` as ephemeral so editor entities
/// never leak into user scene files.
pub fn register_ephemeral_markers_system(resources: &mut Resources) {
    let Some(registry) = resources.get_mut::<EphemeralComponents>() else {
        tracing::warn!(
            "EditorPlugin: EphemeralComponents missing — editor entities WILL leak into scene saves"
        );
        return;
    };
    registry.insert(TypeId::of::<EditorOnly>());
}

/// Startup system: spawns the singleton editor camera entity.
///
/// Idempotent: if a camera entity already carries `EditorCamera`, no new
/// one is spawned. This makes the system safe to register multiple times
/// or to run after a scene reload (which preserves ephemeral entities).
pub fn spawn_editor_camera_system(resources: &mut Resources) {
    if editor_camera_exists(resources) {
        return;
    }

    let controller = resources
        .get::<EditorCameraController>()
        .cloned()
        .unwrap_or_default();

    let transform = initial_transform(&controller);

    let mut commands = match resources.remove::<Commands>() {
        Some(c) => c,
        None => {
            tracing::error!("EditorPlugin: Commands missing — cannot spawn editor camera");
            return;
        }
    };

    commands
        .spawn(resources)
        .insert(EditorCamera)
        .insert(EditorOnly)
        .insert(PerspectiveCamera {
            active: true,
            priority: EDITOR_CAMERA_PRIORITY,
            ..Default::default()
        })
        .insert(transform)
        .insert(StreamingFocus::default());
    commands.apply(resources);
    resources.insert(commands);

    tracing::info!(
        position = ?transform.position,
        priority = EDITOR_CAMERA_PRIORITY,
        "EditorCamera spawned"
    );
}

fn editor_camera_exists(resources: &Resources) -> bool {
    use ome_ecs::archetype_registry::ArchetypeRegistry;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return false;
    };
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    archetypes
        .iter_matching(&[])
        .any(|arch| arch.components().contains(&editor_camera_tid))
}

/// PreRender system: keeps the editor camera's `active` flag in sync
/// with [`PlayState`].
///
/// In edit mode the editor camera owns the viewport (priority 1000
/// outranks gameplay cameras). In play mode it goes inactive so the
/// renderer falls back to the highest-priority *active* camera in the
/// scene, which is the user's gameplay camera.
///
/// Idempotent: only writes when the flag actually changes, so it is
/// cheap to run every frame.
pub fn sync_editor_camera_active_system(resources: &mut Resources) {
    let is_playing = resources
        .get::<PlayState>()
        .map(|ps| ps.is_playing())
        .unwrap_or(false);
    let want_active = !is_playing;

    let Some(entity) = find_editor_camera_entity(resources) else {
        return;
    };

    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<PerspectiveCamera>() else {
        return;
    };
    let Some(cam) = storage.get_mut(entity) else {
        return;
    };
    if cam.active != want_active {
        cam.active = want_active;
    }
}

fn find_editor_camera_entity(resources: &Resources) -> Option<ome_ecs::Entity> {
    use ome_ecs::archetype_registry::ArchetypeRegistry;

    let archetypes = resources.get::<ArchetypeRegistry>()?;
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    for arch in archetypes.iter_matching(&[]) {
        if arch.components().contains(&editor_camera_tid) {
            return arch.entities().first().copied();
        }
    }
    None
}

/// Computes the initial world `Transform` from the controller's defaults.
///
/// Uses a right-handed look-at from `DEFAULT_EYE` toward `focus_point`,
/// then derives the camera's world rotation by inverting the view matrix.
fn initial_transform(controller: &EditorCameraController) -> Transform {
    let view = Mat4::look_at_rh(DEFAULT_EYE, controller.focus_point, Vec3::Y);
    let world = view.inverse();
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    Transform::new(translation, rotation, Vec3::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_transform_places_camera_at_default_eye() {
        let controller = EditorCameraController::default();
        let t = initial_transform(&controller);
        let delta = (t.position - DEFAULT_EYE).length();
        assert!(delta < 1e-4, "expected position {DEFAULT_EYE:?}, got {:?}", t.position);
    }

    #[test]
    fn initial_transform_looks_at_focus_point() {
        let controller = EditorCameraController::default();
        let t = initial_transform(&controller);
        // Camera-forward in glam right-handed view space is -Z, so the
        // world-space forward direction is `rotation * -Z`.
        let forward = (t.rotation * -Vec3::Z).normalize();
        let expected = (controller.focus_point - t.position).normalize();
        let dot = forward.dot(expected);
        assert!(dot > 0.999, "forward {forward:?} should point at focus, dot={dot}");
    }

    #[test]
    fn editor_camera_priority_is_above_default() {
        // PerspectiveCamera default priority is 0; editor must override.
        assert!(EDITOR_CAMERA_PRIORITY > 0);
    }
}
