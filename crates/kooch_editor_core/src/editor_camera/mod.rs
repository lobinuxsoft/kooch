//! Editor camera — independent viewport navigation.
//!
//! Spawns one ECS entity tagged with [`EditorCamera`] + [`EditorOnly`] +
//! `PerspectiveCamera` + `Transform` at editor startup. The `EditorOnly`
//! marker is registered in
//! [`EphemeralComponents`](kooch_ecs::EphemeralComponents) so the entity is
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

use kooch_core::resource::Resources;
use kooch_ecs::EphemeralComponents;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
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
///
/// Per design freeze of issue #371 (Q4): the eye sits inside cascade 0
/// (the GDF cascade with 0.25 m voxel size, 16 m cube around the
/// active origin) at startup. `(0, 5, 8)` looking at `Vec3::ZERO` keeps
/// the eye + focus point inside cascade 0 for any sensible orbit
/// distance.
const DEFAULT_EYE: Vec3 = Vec3::new(0.0, 5.0, 8.0);

/// Startup system: registers the editor's non-scene markers as ephemeral
/// so editor entities never leak into user scene files.
///
/// - `EditorOnly` — the editor's own furniture (camera, gizmos).
/// - `MirrorEntity` — the local stand-ins for a remote project's world.
///   Those entities belong to the project, which saves them itself; a
///   local save must not write the mirror over the project's scene.
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
///
/// 🔴 Keeps looking past an archetype that carries the marker but holds
/// no entities. The previous version returned from inside the `if`, so
/// the first matching archetype decided the answer — and an archetype
/// the camera has *left* still lists the marker. Gaining or losing any
/// component migrates the entity and leaves that empty shell behind, at
/// which point every caller was told the editor camera did not exist:
/// 2650 viewport deltas produced, none applied, the camera frozen at its
/// spawn pose until something happened to reorder the archetypes.
///
/// One implementation, here with the marker it looks for. There used to
/// be a second identical copy in `input::apply`, which is how a bug like
/// this survives a reading.
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
        assert!(
            delta < 1e-4,
            "expected position {DEFAULT_EYE:?}, got {:?}",
            t.position
        );
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
        assert!(
            dot > 0.999,
            "forward {forward:?} should point at focus, dot={dot}"
        );
    }

    #[test]
    fn editor_camera_priority_is_above_default() {
        // PerspectiveCamera default priority is 0; editor must override.
        assert!(EDITOR_CAMERA_PRIORITY > 0);
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;
    use kooch_ecs::allocator::EntityAllocator;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::perspective_camera::PerspectiveCamera;
    use kooch_ecs::transform::Transform;

    /// An archetype the camera has *left* still lists the marker.
    ///
    /// This is what froze the editor camera: the lookup returned from
    /// inside the loop, so the first archetype carrying `EditorCamera`
    /// decided the answer even when it held no entities. Adding or
    /// removing any component on the camera produces exactly this shape.
    #[test]
    fn an_empty_archetype_with_the_marker_does_not_hide_the_camera() {
        let mut resources = Resources::new();
        let mut alloc = EntityAllocator::new();
        let camera = alloc.spawn();
        resources.insert(alloc);

        let mut archetypes = ArchetypeRegistry::new();

        // The shell left behind: carries the marker, holds nobody.
        let abandoned: std::collections::BTreeSet<_> = [
            TypeId::of::<EditorCamera>(),
            TypeId::of::<PerspectiveCamera>(),
        ]
        .into_iter()
        .collect();
        archetypes.get_or_create(abandoned);

        // Where the camera actually lives now.
        let current: std::collections::BTreeSet<_> = [
            TypeId::of::<EditorCamera>(),
            TypeId::of::<PerspectiveCamera>(),
            TypeId::of::<Transform>(),
        ]
        .into_iter()
        .collect();
        let current_id = archetypes.get_or_create(current);
        archetypes.register_entity(camera, current_id);
        resources.insert(archetypes);

        assert_eq!(
            find_editor_camera_entity(&resources),
            Some(camera),
            "the lookup stopped at the empty archetype and reported no camera",
        );
    }

    #[test]
    fn no_camera_at_all_is_none() {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ArchetypeRegistry::new());
        assert_eq!(find_editor_camera_entity(&resources), None);
    }
}
