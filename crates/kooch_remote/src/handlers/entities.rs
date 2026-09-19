//! Entity and component requests: resolve, spawn, edit fields and components, reparent, despawn.

use super::*;

/// Resolves a live entity handle, erroring if it is not alive.
pub(super) fn resolve_entity(resources: &Resources, id: EntityId) -> Result<Entity, RemoteError> {
    let entity = Entity::from(id);
    let alive = resources
        .get::<EntityAllocator>()
        .is_some_and(|a| a.is_alive(entity));
    if alive {
        Ok(entity)
    } else {
        Err(RemoteError::NoSuchEntity { entity: id })
    }
}

/// Captures `entity` and its descendants into a scene file.
pub(super) fn save_prefab(
    resources: &mut Resources,
    entity: EntityId,
    path: &str,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let document = SceneDocument::from_ecs_subtree(resources, entity);
    // A prefab file promises exactly one root — the invariant its extension
    // names. Enforced on write so it cannot be discovered at the click that
    // instances it.
    document.root_index().map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    // Writes the `.meta` alongside, so the prefab is a registered asset the
    // moment it exists rather than the first time something loads it.
    kooch_ecs::scene::prefab::save(&document, path.as_ref())
        .map(|_| ())
        .map_err(|e| RemoteError::SceneError {
            detail: e.to_string(),
        })
}

/// Brings the project's copy of an asset back in line with the disk: overwritten in place so
/// existing handles see it, and registered so a new file's guid resolves.
pub(super) fn reload_asset(resources: &mut Resources, path: &str) {
    let written = kooch_core::asset_loader::asset_written(path.as_ref(), resources);
    tracing::info!(
        target: "kooch_remote::assets",
        %path,
        reloaded = written.reloaded,
        registered = written.guid.is_some(),
        "asset written by the editor",
    );
}

/// Stamps a prefab file into the live world and hands back its root.
pub(super) fn instantiate_prefab(
    resources: &mut Resources,
    path: &str,
) -> Result<EntityId, RemoteError> {
    let prefab = SceneDocument::load(path.as_ref()).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;

    // The instance belongs to the scene being edited. With no scene active
    // it becomes its own — a fresh guid rather than a shared sentinel,
    // which two unrelated instances would collide inside.
    let into = resources
        .get::<kooch_ecs::SceneManager>()
        .and_then(|scenes| scenes.active_id())
        .unwrap_or_else(kooch_core::Guid::new_v4);

    let (root, members) =
        kooch_ecs::scene::instantiate_members(&prefab, resources, into).map_err(|e| {
            RemoteError::SceneError {
                detail: e.to_string(),
            }
        })?;

    // The prefab link is attached here because this method *is* the editor's instancing; a game's
    // `spawn_prefab` attaches nothing.
    match kooch_core::asset_meta::read_meta(path.as_ref()) {
        Ok(meta) => {
            kooch_ecs::prefab_instance::attach(resources, root, &members, meta.guid);
            tracing::info!(
                target: "kooch_remote::prefab",
                prefab = %meta.guid,
                members = members.len(),
                "instance linked to its prefab",
            );
        }
        // Without an identity there is nothing to link *to*. Said out loud
        // because the instance still spawns, so the only visible symptom
        // is that it never follows the prefab afterwards.
        Err(e) => tracing::warn!(
            target: "kooch_remote::prefab",
            path = %path,
            "instanced but not linked, no asset identity: {e}",
        ),
    }
    Ok(EntityId::from(root))
}

/// Resolves a component name to a local `TypeId`.
pub(super) fn resolve_component(
    resources: &Resources,
    type_name: &str,
) -> Result<TypeId, RemoteError> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.type_id_by_name(type_name))
        .ok_or_else(|| RemoteError::UnknownComponent {
            type_name: type_name.to_owned(),
        })
}

pub(super) fn set_field(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
    field: &str,
    value: kooch_ecs::reflect::ReflectValue,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;
    let registry =
        resources
            .get_mut::<ComponentRegistry>()
            .ok_or_else(|| RemoteError::Unavailable {
                detail: "no ComponentRegistry".into(),
            })?;
    registry
        .reflect_set_field(&type_id, entity, field, value)
        .map_err(|e| RemoteError::FieldError {
            detail: e.to_string(),
        })
}

pub(super) fn add_component(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;

    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
    Ok(())
}

pub(super) fn remove_component(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(entity, &type_id);
    }
    update_archetype_remove(resources, entity, type_id);
    Ok(())
}

/// Spawns an entity with what every authored entity carries: `Name` and `Transform`, named or not,
/// as the editor's local spawn does.
pub(super) fn spawn(
    resources: &mut Resources,
    name: Option<&str>,
    scene: Option<kooch_core::Guid>,
    parent: Option<EntityId>,
) -> Entity {
    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    let entity = commands.spawn(resources).id();
    resources.insert(commands);

    add_default(resources, entity, TypeId::of::<Name>());
    add_default(resources, entity, TypeId::of::<Transform>());

    if let Some(name) = name
        && let Some(storage) = resources
            .get_mut::<ComponentRegistry>()
            .and_then(|r| r.get_cpu_mut::<Name>())
        && let Some(n) = storage.get_mut(entity)
    {
        n.value = name.to_owned();
    }

    if let Some(parent) = parent {
        let _ = set_parent(resources, entity.into(), Some(parent));
    }
    // The parent's scene wins: an entity's scene *is* its parent's, so a
    // child authored into a different one would be written to a file its
    // parent is not in and come back an orphan.
    let home = parent
        .and_then(|parent| scene_of(resources, parent))
        .or(scene)
        .or_else(|| {
            resources
                .get::<kooch_ecs::SceneManager>()
                .and_then(|manager| manager.active_id())
        });
    if let Some(home) = home {
        tag_with_scene(resources, entity, home);
    }
    entity
}

/// Records which scene a newly spawned entity belongs to. 🔴 Without it the entity is Unsaved until
/// a save adopts it into the active scene.
pub(super) fn tag_with_scene(resources: &mut Resources, entity: Entity, scene: kooch_core::Guid) {
    use kooch_ecs::SceneMember;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    update_archetype_add(resources, entity, TypeId::of::<SceneMember>());
}

/// Inserts `type_id`'s default on `entity` and moves it to its new archetype; an unregistered type
/// is skipped, not fatal.
pub(super) fn add_default(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
}

/// Reparents an entity, or unparents with `None`, through `kooch_ecs::hierarchy::reparent` — the
/// editor's own code, keeping the world transform.
pub(super) fn set_parent(
    resources: &mut Resources,
    entity: EntityId,
    parent: Option<EntityId>,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    // Resolved before the call so an unknown parent is reported rather than
    // silently unparenting the child to the root.
    let parent = match parent {
        Some(parent) => Some(resolve_entity(resources, parent)?),
        None => None,
    };
    kooch_ecs::hierarchy::reparent(resources, entity, parent);
    Ok(())
}

/// Despawns an entity **and everything under it**, or children survive with a dead `Parent` and
/// accumulate in the saved scene.
pub(super) fn despawn(resources: &mut Resources, entity: EntityId) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;

    // Collected before anything is despawned: the walk reads `Children`,
    // and despawning as it goes would cut the branch it is standing on.
    let doomed = match resources.get::<ComponentRegistry>() {
        Some(registry) => kooch_ecs::hierarchy::collect_descendants(entity, &registry),
        None => vec![entity],
    };

    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    for entity in doomed {
        commands.despawn(entity);
    }
    commands.apply(resources);
    resources.insert(commands);
    Ok(())
}
