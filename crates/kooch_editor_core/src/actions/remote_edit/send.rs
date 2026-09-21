//! Sending one classified edit to the project's server.

use super::*;

/// Sends one [`Edit`] to the project's server.
pub(super) fn send(
    edit: Edit<'_>,
    session: &crate::remote_session::RemoteSession,
    mirror: &crate::remote_mirror::RemoteMirror,
    names: Option<&ComponentNames>,
    resources: &Resources,
    created: &mut Vec<kooch_remote::protocol::EntityId>,
) -> Result<(), String> {
    use kooch_ecs::reflect::ReflectValue;

    let client = session.client();
    // Maps a local mirror entity to the remote id the server addresses.
    let remote = |e| {
        mirror
            .remote_of(e)
            .ok_or_else(|| "entity not in mirror".to_owned())
    };
    // Resolves a portable component id to the type name the server keys by.
    let name = |c| {
        names
            .and_then(|n| n.name(c))
            .map(str::to_owned)
            .ok_or_else(|| "component id not interned".to_owned())
    };
    let map_err = |e: kooch_remote::ClientError| e.to_string();

    match edit {
        Edit::SetField {
            entity,
            component,
            field,
            value,
        } => client
            .set_field(
                remote(entity)?,
                &name(component)?,
                field,
                to_remote_value(value.clone(), mirror)?,
            )
            .map_err(map_err),
        Edit::AddComponent { entity, component } => client
            .add_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::RemoveComponent { entity, component } => client
            .remove_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::Despawn(entity) => client.despawn(remote(entity)?).map_err(map_err),
        Edit::Reparent { entity, new_parent } => {
            let parent = match new_parent {
                Some(parent) => Some(remote(parent)?),
                None => None,
            };
            client.set_parent(remote(entity)?, parent).map_err(map_err)
        }
        Edit::Duplicate(entity) => {
            created.push(duplicate(entity, &client, mirror, resources)?);
            Ok(())
        }
        Edit::MoveToScene { entity, scene } => {
            let id = remote(entity)?;
            // Already there for anything that belongs to a scene, and an error here is not a
            // failure: the set below is the edit, and it needs the component to exist however it
            // got there. 🔴 The FULL path.
            let member = std::any::type_name::<kooch_ecs::SceneMember>();
            if let Err(e) = client.add_component(id, member) {
                tracing::debug!(
                    target: "kooch_editor_core::remote_edit::move_to_scene",
                    "SceneMember was not added, assuming it is already there: {e}",
                );
            }
            client
                .set_field(
                    id,
                    member,
                    "scene",
                    kooch_ecs::reflect::ReflectValue::String(scene.to_string()),
                )
                .map_err(map_err)
        }
        Edit::Paste { into, states } => {
            // 🔴 Once for the whole paste. `NewScene` asks the project to
            // make one, and asking per entity would hand a clipboard of
            // five entities five scenes holding one each.
            let scene = match into {
                crate::actions::SpawnTarget::Active => None,
                crate::actions::SpawnTarget::Scene(id) => Some(id),
                // Not reachable from the panel — nothing offers "paste as a child of" — and the
                // mirror has no scene lookup to answer it with. Treated as the active scene rather
                // than invented.
                crate::actions::SpawnTarget::ChildOf(_) => None,
                crate::actions::SpawnTarget::NewScene => Some(client.new_scene().map_err(map_err)?),
            };
            for state in &states {
                created.push(build(
                    &client,
                    mirror,
                    &crate::actions::entity_state::as_copy(state),
                    scene,
                )?);
            }
            Ok(())
        }
        Edit::Spawn {
            name: entity_name,
            extra,
            into,
        } => {
            // Asked for, not inferred. A menu opened on a scene or an entity that is not the active
            // one means *there*, and a spawn that lands in the active scene instead shows up as a
            // row in the wrong group with nothing saying why.
            let (scene, parent) = match into {
                crate::actions::SpawnTarget::Active => (None, None),
                crate::actions::SpawnTarget::Scene(id) => (Some(id), None),
                crate::actions::SpawnTarget::ChildOf(local) => (None, remote(local).ok()),
                // Two calls, not a flag on the spawn. The project owns the open set, so creating a
                // scene is its answer to give — and the id it hands back is what the entity is then
                // authored into.
                crate::actions::SpawnTarget::NewScene => {
                    (Some(client.new_scene().map_err(map_err)?), None)
                }
            };
            let entity = client
                .spawn(entity_name.as_deref(), scene, parent)
                .map_err(map_err)?;
            created.push(entity);
            // Remote `spawn` creates only `Name`, while the local path adds Name + Transform +
            // extras. Everything past the name has to be asked for explicitly, or the entity
            // arrives inert — a light with no Transform has no position and no direction.
            let transform = std::any::type_name::<kooch_ecs::transform::Transform>();
            client.add_component(entity, transform).map_err(map_err)?;

            // `extra` is a list of local `TypeId`s; the server keys
            // components by name, and the registry is the only thing that
            // knows the mapping.
            let registry = resources.get::<kooch_ecs::component::ComponentRegistry>();
            for type_id in extra {
                let Some(component_name) =
                    registry.as_ref().and_then(|r| r.component_name(&type_id))
                else {
                    // Nothing to send and nothing to guess: a type the
                    // local registry has never seen has no name the server
                    // would recognise.
                    tracing::warn!(
                        target: "kooch_editor_core::remote_edit",
                        ?type_id,
                        "spawn requested a component with no registered name",
                    );
                    continue;
                };
                if component_name == transform {
                    continue;
                }
                client
                    .add_component(entity, component_name)
                    .map_err(map_err)?;
            }
            Ok(())
        }
        Edit::TransformEdit { entity, transform } => {
            let id = remote(entity)?;
            let ty = std::any::type_name::<kooch_ecs::transform::Transform>();
            // A gizmo drag replaces the whole transform; push each field.
            for (field, value) in [
                ("position", ReflectValue::Vec3(transform.position)),
                ("rotation", ReflectValue::Quat(transform.rotation)),
                ("scale", ReflectValue::Vec3(transform.scale)),
            ] {
                client.set_field(id, ty, field, value).map_err(map_err)?;
            }
            Ok(())
        }
        // Both processes see the same filesystem, so a path from the mirrored open set, or one
        // picked here, is meaningful on the project's side of the wire.
        Edit::SaveScene { as_new } => {
            let scenes = session.open_scenes().unwrap_or_default();
            let active = scenes.iter().find(|s| s.active);
            save_one(
                client,
                resources,
                active.map(|s| s.id),
                active.and_then(|s| s.path.as_deref()),
                as_new,
            )
        }
        Edit::SaveOneScene { scene, as_new } => {
            let scenes = session.open_scenes().unwrap_or_default();
            let path = scenes
                .iter()
                .find(|s| s.id == scene)
                .and_then(|s| s.path.as_deref());
            save_one(client, resources, Some(scene), path, as_new)
        }
        Edit::MoveEntity {
            entity,
            parent,
            before,
        } => {
            // One call. The numbering lives on the project, so a client
            // doing it would renumber a sibling group over the wire — one
            // round trip per entity, for a drag.
            let entity = remote(entity)?;
            let parent = parent.map(remote).transpose()?;
            let before = before.map(remote).transpose()?;
            client.move_entity(entity, parent, before).map_err(map_err)
        }
        Edit::RevertOneScene(scene) => client.revert_scene(Some(scene)).map_err(map_err),
        Edit::LoadScene { path } => match named_or_asked(resources, path) {
            Some(path) => client.load_scene(&path.to_string_lossy()).map_err(map_err),
            None => Ok(()),
        },
        Edit::CloseScene(scene) => client.close_scene(scene).map_err(map_err),
        Edit::SetActiveScene(scene) => client.set_active_scene(scene).map_err(map_err),
        Edit::LoadSceneAdditive { path } => match named_or_asked(resources, path) {
            Some(path) => client
                .load_scene_additive(&path.to_string_lossy())
                .map(|scene| {
                    tracing::info!(%scene, "scene opened additively in the project");
                })
                .map_err(map_err),
            None => Ok(()),
        },
        Edit::SetPlaying(playing) => client.set_playing(playing).map_err(map_err),
        Edit::SetSystemEnabled { name, nth, enabled } => client
            .set_system_enabled(&name, nth, enabled)
            .map_err(map_err),
        // Sent as ordinary field writes, but *not* as `EditorAction`s: an edit on an instance is
        // recorded as an override, so routing propagation through the action layer would pin every
        // field it touched and the instance would stop following the prefab.
        Edit::RevertToPrefab {
            root,
            overrides,
            writes,
        } => {
            push_writes(client, &writes, &remote)?;
            let id = remote(root)?;
            client
                .set_field(
                    id,
                    std::any::type_name::<kooch_ecs::prefab_instance::PrefabInstance>(),
                    "overrides",
                    ReflectValue::String(overrides),
                )
                .map_err(map_err)
        }
        Edit::ReloadAssetOnHost(path) => client
            .reload_asset(&path.to_string_lossy())
            .map_err(map_err),
        Edit::PropagatePrefab(writes, removals) => {
            for removal in &removals {
                let id = remote(removal.entity)?;
                if let Err(e) = client.remove_component(id, &removal.component) {
                    tracing::warn!(
                        "prefab propagation could not remove {}: {e}",
                        removal.component,
                    );
                }
            }
            push_writes(client, &writes, &remote)
        }
        // Both processes see the same filesystem, so a path resolved here
        // is meaningful on the project's side of the wire — the same
        // assumption scene I/O above already makes.
        Edit::SavePrefab { entity, dest } => {
            let id = remote(entity)?;
            let Some(root) = crate::actions::handlers::prefab_root(resources) else {
                return Err("cannot save a prefab without a project open".to_owned());
            };
            // The mirror's `Name` is the project's `Name`; reading it here
            // saves a round trip purely to learn what to call the file.
            let name = crate::actions::handlers::entity_name(resources, entity);
            let path = crate::actions::handlers::prefab_path(&root, &name, dest.as_deref());
            client
                .save_prefab(id, &path.to_string_lossy())
                .map_err(map_err)
        }
        Edit::InstantiatePrefab { path, at } => {
            let root = client
                .instantiate_prefab(&path.to_string_lossy())
                .map_err(map_err)?;
            created.push(root);
            // Placing the instance is a `SetField` on the root that just came back, rather than a
            // parameter on the call. It reuses the path that already knows how to write a reflected
            // field, and keeps spatial types out of the wire format.
            let Some(at) = at else {
                return Ok(());
            };
            client
                .set_field(
                    root,
                    std::any::type_name::<kooch_ecs::transform::Transform>(),
                    "position",
                    ReflectValue::Vec3(at),
                )
                .map_err(map_err)
        }
    }
}

/// Saves one of the project's scenes to its own file, or to one asked for when `as_new` or never
/// saved. `None` is the project's active scene.
fn save_one(
    client: &kooch_remote::RemoteClient,
    resources: &Resources,
    scene: Option<kooch_core::Guid>,
    known: Option<&str>,
    as_new: bool,
) -> Result<(), String> {
    use crate::actions::scene_io::{picked, scene_dialog};
    let path = match known.filter(|_| !as_new) {
        Some(path) => path.to_owned(),
        None => {
            let dialog = scene_dialog(resources, known.map(std::path::Path::new));
            match picked(dialog.save_file()) {
                Some(path) => path.to_string_lossy().into_owned(),
                None => return Ok(()),
            }
        }
    };
    client.save_scene(&path, scene).map_err(|e| e.to_string())?;
    tracing::info!("scene saved to {path}");
    Ok(())
}
