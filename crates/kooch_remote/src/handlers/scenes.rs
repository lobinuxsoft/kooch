//! Scene requests: load, close, save, activate, revert, and moving entities between scenes.

use super::*;

pub(super) fn load_scene(resources: &mut Resources, path: &str) -> Result<(), RemoteError> {
    match load_through_manager(resources, path) {
        Some(result) => result?,
        None => load_directly(resources, path)?,
    }
    // Refreshes prefab copies left stale while this scene was closed — here, where the scene
    // arrives.
    kooch_ecs::scene::propagate::refresh_all(resources);
    Ok(())
}

/// Closes one open scene, despawning only its entities; asking about unsaved edits is the editor's
/// job.
pub(super) fn close_scene(
    resources: &mut Resources,
    scene: kooch_core::Guid,
) -> Result<(), RemoteError> {
    let mut manager = resources
        .remove::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?;
    let closed = manager.close(scene, resources);
    resources.insert(manager);
    match closed {
        true => Ok(()),
        false => Err(RemoteError::SceneError {
            detail: format!("scene {scene} is not open"),
        }),
    }
}

/// Points the active slot at an already-open scene.
pub(super) fn set_active_scene(
    resources: &mut Resources,
    scene: kooch_core::Guid,
) -> Result<(), RemoteError> {
    let known = resources
        .get_mut::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?
        .set_active(scene);
    match known {
        true => Ok(()),
        false => Err(RemoteError::SceneError {
            detail: format!("scene {scene} is not open"),
        }),
    }
}

/// Opens a scene beside the loaded ones; unlike [`load_scene`] nothing is despawned, so client
/// handles stay valid.
pub(super) fn load_scene_additive(
    resources: &mut Resources,
    path: &str,
) -> Result<kooch_core::Guid, RemoteError> {
    let path = std::path::PathBuf::from(path);
    let mut manager = resources
        .remove::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?;
    let opened = manager.open_additive(&path, resources);
    resources.insert(manager);
    let scene = opened.map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    // Same reason as `load_scene`: a prefab edited while this scene was
    // closed left stale copies in it, and this is where it arrives.
    kooch_ecs::scene::propagate::refresh_all(resources);
    Ok(scene)
}

/// Marks dirty the scene holding `entity` — its own scene, not the active one. 🔴 Before this
/// nothing marked a scene dirty, so the asterisk never appeared.
pub(super) fn touch_entity(resources: &mut Resources, entity: EntityId) {
    let scene = scene_of(resources, entity);
    touch_scene(resources, scene);
}

/// Which scene an entity belongs to, or `None` for one that belongs to
/// none — spawned here and not yet adopted by a save.
pub(super) fn scene_of(resources: &Resources, entity: EntityId) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<kooch_ecs::SceneMember>()?
        .get(Entity::from(entity))
        .map(|member| member.scene)
}

/// Marks one scene dirty, or the active one when the entity belonged to
/// none.
pub(super) fn touch_scene(resources: &mut Resources, scene: Option<kooch_core::Guid>) {
    let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() else {
        return;
    };
    match scene {
        // A scene the project does not have open is not this host's to
        // record. `mark_scene_dirty` says so; nothing here can act on it.
        Some(id) => {
            manager.mark_scene_dirty(id);
        }
        None => manager.mark_dirty(),
    }
}

/// Writes one open scene to `path` through the project's manager, so it adopts the path and clears
/// dirty. 🔴 One scene, not the world, keeping its id. `None` saves the active one.
pub(super) fn save_scene(
    resources: &mut Resources,
    path: &str,
    scene: Option<kooch_core::Guid>,
) -> Result<(), RemoteError> {
    let Some(mut manager) = resources.remove::<kooch_ecs::SceneManager>() else {
        // Refused rather than falling back to writing everything alive:
        // that fallback is the bug this function exists to remove, and a
        // silent one is worse than an error naming what is missing.
        return Err(RemoteError::Unavailable {
            detail: "no SceneManager; nothing knows which scene to write".into(),
        });
    };
    let result = match scene.or_else(|| manager.active_id()) {
        Some(id) => manager
            .save_scene_as(id, std::path::PathBuf::from(path), resources)
            .map_err(|e| RemoteError::SceneError {
                detail: e.to_string(),
            }),
        None => Err(RemoteError::SceneError {
            detail: "no scene is open".into(),
        }),
    };
    resources.insert(manager);
    result
}

/// Moves an entity among its siblings, through the engine's own policy.
pub(super) fn move_entity(
    resources: &mut Resources,
    entity: EntityId,
    parent: Option<EntityId>,
    before: Option<EntityId>,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let parent = parent.map(|p| resolve_entity(resources, p)).transpose()?;
    // A `before` that is no longer alive means "last", not an error: the
    // client is describing a list it read a frame ago, and refusing would
    // turn a stale row into a failed drag.
    let before = before.map(Entity::from).filter(|e| {
        resources
            .get::<EntityAllocator>()
            .is_some_and(|a| a.is_alive(*e))
    });

    match kooch_ecs::order::place(resources, entity, parent, before) {
        true => Ok(()),
        // The one refusal `place` makes: into its own subtree, which
        // would detach that subtree from the world.
        false => Err(RemoteError::FieldError {
            detail: "an entity cannot be moved into its own subtree".into(),
        }),
    }
}

/// Throws away one scene's edits and reads it back from its file, with the manager lifted out as a
/// load does.
pub(super) fn revert_scene(
    resources: &mut Resources,
    scene: Option<kooch_core::Guid>,
) -> Result<(), RemoteError> {
    let Some(mut manager) = resources.remove::<kooch_ecs::SceneManager>() else {
        return Err(RemoteError::Unavailable {
            detail: "no SceneManager; nothing knows which scene to revert".into(),
        });
    };
    let result = match scene.or_else(|| manager.active_id()) {
        Some(id) => manager
            .revert(id, resources)
            .map_err(|e| RemoteError::SceneError {
                detail: e.to_string(),
            }),
        None => Err(RemoteError::SceneError {
            detail: "no scene is open".into(),
        }),
    };
    resources.insert(manager);
    // A prefab edited while this scene held stale copies of it: the
    // entities were just respawned from the file, so they need the same
    // refresh a load gives them.
    if result.is_ok() {
        kooch_ecs::scene::propagate::refresh_all(resources);
    }
    result
}

/// Loads through the project's [`SceneManager`](kooch_ecs::SceneManager), so its record matches the
/// world — straight to the ECS, the second scene opened kept the first one's name. `None` without a
/// manager.
pub(super) fn load_through_manager(
    resources: &mut Resources,
    path: &str,
) -> Option<Result<(), RemoteError>> {
    // Lifted out and put back: `load` needs `&mut Resources` for the ECS
    // it is about to replace, and the manager lives in there too.
    let mut manager = resources.remove::<kooch_ecs::SceneManager>()?;
    let result = manager.load(path.as_ref(), resources);
    resources.insert(manager);
    Some(result.map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    }))
}

/// Loads without a manager: the entities arrive, nothing records them.
pub(super) fn load_directly(resources: &mut Resources, path: &str) -> Result<(), RemoteError> {
    let doc = SceneDocument::load(path.as_ref()).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    sync_scene_to_ecs(&doc, resources).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })
}

/// Drops both diff caches so the next reply describes the whole world. 🔴 A restore is invisible to
/// a diff, and the cache answering after Stop never saw play (#1035).
pub(super) fn forget_the_world(resources: &mut Resources) {
    resources.remove::<crate::snapshot_cache::SnapshotCache>();
    resources.remove::<crate::moved_cache::MovedCache>();
}
