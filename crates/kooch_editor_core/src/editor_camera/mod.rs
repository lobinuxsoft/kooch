//! Editor camera — independent viewport navigation.

pub mod controller;
pub mod fly;
pub(crate) mod framing;
pub mod input;
pub mod markers;
pub mod orbit;
pub mod pan_zoom;

use std::any::TypeId;

use glam::Vec3;

use kooch_core::resource::Resources;
use kooch_ecs::EphemeralComponents;
use kooch_ecs::commands::Commands;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::transform::Transform;
use kooch_world::focus::StreamingFocus;

pub use controller::EditorCameraController;
pub use markers::{EditorCamera, EditorOnly};

/// Priority assigned to the editor camera's `PerspectiveCamera`. Chosen
/// well above any plausible gameplay camera priority so the editor view
/// always wins while the marker is `active`.
pub const EDITOR_CAMERA_PRIORITY: i32 = 1000;

/// Default world-space spawn position for the editor camera.
const DEFAULT_EYE: Vec3 = Vec3::new(0.0, 5.0, 8.0);

/// Startup system: registers the editor's non-scene markers as ephemeral so editor entities never
/// leak into user scene files.
pub fn register_ephemeral_markers_system(resources: &mut Resources) {
    let Some(registry) = resources.get_mut::<EphemeralComponents>() else {
        tracing::warn!(
            "EditorPlugin: EphemeralComponents missing — editor entities WILL leak into scene saves"
        );
        return;
    };
    registry.insert(TypeId::of::<EditorOnly>());
    registry.insert(TypeId::of::<crate::remote_mirror::MirrorEntity>());
}

/// Startup system: spawns the singleton editor camera entity.
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
    use kooch_ecs::archetype_registry::ArchetypeRegistry;

    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return false;
    };
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    archetypes
        .iter_matching(&[])
        .any(|arch| arch.components().contains(&editor_camera_tid))
}

/// The editor camera's entity, or `None` if it has not been spawned.
pub(crate) fn editor_camera_rotation(resources: &Resources) -> Option<glam::Quat> {
    let entity = find_editor_camera_entity(resources)?;
    let registry = resources.get::<kooch_ecs::ComponentRegistry>()?;
    let storage = registry.get_cpu::<kooch_ecs::Transform>()?;
    Some(storage.get(entity)?.rotation)
}

pub(crate) fn find_editor_camera_entity(resources: &Resources) -> Option<kooch_ecs::Entity> {
    use kooch_ecs::archetype_registry::ArchetypeRegistry;

    let archetypes = resources.get::<ArchetypeRegistry>()?;
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    archetypes
        .iter_matching(&[])
        .filter(|arch| arch.components().contains(&editor_camera_tid))
        .find_map(|arch| arch.entities().first().copied())
}

/// Computes the initial world `Transform` from the controller's defaults.
fn initial_transform(controller: &EditorCameraController) -> Transform {
    let view = glam::camera::rh::view::look_at_mat4(DEFAULT_EYE, controller.focus_point, Vec3::Y);
    let world = view.inverse();
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    Transform::new(translation, rotation, Vec3::ONE)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod find_tests;
