//! The remote sink of the dual-sink edit dispatch.

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentNames;

use crate::actions::EditorAction;
use crate::remote_session::RemoteState;

/// Attempts to handle `action` over the wire.
pub(crate) fn dispatch(resources: &mut Resources, action: &EditorAction) -> bool {
    // Undo/Redo travel as inverses of the edits already sent — the local command stack describes
    // the mirror, which the next refresh overwrites. See [`crate::actions::remote_undo`].
    if let EditorAction::Undo(document) | EditorAction::Redo(document) = action
        && document.is_world()
    {
        crate::actions::remote_undo::step(resources, matches!(action, EditorAction::Undo(_)));
        return true;
    }

    // Spawning a mesh is the one edit that cannot be reduced to a single protocol call: the editor
    // has to load the asset to learn its GUID, and loading mutates the `AssetServer`, which `send`
    // cannot do from an immutable world. Handled here, before `classify`.
    if let EditorAction::SpawnMesh { path, name } = action {
        spawn_mesh(resources, path, name);
        return true;
    }

    // And a block, for the same reason twice over: it writes an asset and resolves its GUID, both
    // of which need a mutable world.
    if let EditorAction::BlockEdit { source, before, .. } = action {
        crate::actions::remote_undo::record_step(
            resources,
            "Edit Block",
            crate::actions::remote_undo::Inverse::BlockShape {
                source: *source,
                shape: before.clone(),
            },
        );
        return true;
    }

    if let EditorAction::SpawnBlock { shape, .. } = action {
        spawn_block(resources, *shape);
        return true;
    }

    // Non-ECS actions stay on the local path even in remote mode: closing
    // the project, toggling power profiles all act on the editor, not the
    // remote world.
    let Some(edit) = classify(action, resources) else {
        // 🔴 A world edit that reaches here is a bug, not a local action. It falls through to
        // `apply_non_ecs_action`, which does not know it either, and the gesture does nothing at
        // all — which is how SpawnBlock and BlockEdit each shipped broken.
        if action.is_a_world_edit() {
            tracing::error!(
                target: "kooch_editor_core::remote_edit",
                "this edits the world and has no route over the wire; it will be dropped",
            );
        }
        return false;
    };

    // Lifted out of Resources so the send can borrow the session and the
    // rest of the world at the same time, and record play state after.
    let Some(mut state) = resources.remove::<RemoteState>() else {
        return true;
    };
    let playing = matches!(edit, Edit::SetPlaying(playing) if playing);
    let is_play_toggle = matches!(edit, Edit::SetPlaying(_));
    // 🔴 The panel draws the CACHED list, and the cache is only pulled once per connection. Without
    // a re-read the project switches the system off and the checkbox springs straight back, which
    // reads as the toggle not working at all (#982).
    let is_system_toggle = matches!(edit, Edit::SetSystemEnabled { .. });

    // Recomputed before the send, which consumes `edit`. Deterministic —
    // the same inputs that produced the path the project is about to write.
    let saved_prefab = match &edit {
        Edit::SavePrefab { entity, dest } => {
            crate::actions::handlers::prefab_root(resources).map(|root| {
                let name = crate::actions::handlers::entity_name(resources, *entity);
                crate::actions::handlers::prefab_path(&root, &name, dest.as_deref())
            })
        }
        _ => None,
    };

    // Asked before the send, because after it the state it describes is
    // gone: an undo needs the value that is about to be overwritten, the
    // component about to be removed, the subtree about to be despawned.
    let before = crate::actions::remote_undo::capture_before(action, resources, &state.mirror);

    let mut sent = false;
    // Entities the edit brought into being. Filled by the arms that
    // create — it is the only thing an undo of a creation needs, and the
    // ids exist nowhere until the project answers with them.
    let mut created: Vec<kooch_remote::protocol::EntityId> = Vec::new();
    if let Some(session) = state.session.as_ref() {
        let names = resources.get::<ComponentNames>();
        match send(edit, session, &state.mirror, names, resources, &mut created) {
            Ok(()) if is_play_toggle => {
                state.playing = playing;
                sent = true;
            }
            Ok(()) => sent = true,
            Err(e) => tracing::warn!("remote edit dropped: {e}"),
        }
    }

    // Read back before the state goes home: the panel should show what
    // the project did, not what it was asked to do.
    if sent
        && is_system_toggle
        && let Some(session) = state.session.as_mut()
    {
        session.refresh_systems();
    }

    resources.insert(state);

    if sent {
        // Selecting what was just made — but only for a creation the user asked for. Undoing a
        // despawn also creates, and stealing the selection there would fight whatever they had
        // selected when they pressed Ctrl+Z.
        if selects_what_it_makes(action)
            && !created.is_empty()
            && let Some(state) = resources.get_mut::<RemoteState>()
        {
            state.pending_selection = created.clone();
        }
        crate::actions::remote_undo::record(resources, action, before, created);
        // The editor is waiting to see this one, so it does not wait out
        // the half-second poll to find out.
        pull_soon(resources);
    }

    // The project wrote the file; this side has to be told it exists, or the Inspector cannot find
    // what the user just made until the editor restarts. Done here rather than in `send`, which
    // holds the world immutably so it can borrow the session alongside it.
    if let Some(path) = saved_prefab.filter(|_| sent) {
        crate::actions::handlers::asset_saved(resources, &path);
        // The project wrote bytes this side's cache has never seen.
        crate::actions::handlers::prefab_saved(resources, &path);
    }
    true
}

/// Whether the entities this action creates should end up selected.
fn selects_what_it_makes(action: &EditorAction) -> bool {
    matches!(
        action,
        EditorAction::Duplicate(_)
            | EditorAction::PasteEntities { .. }
            | EditorAction::Spawn { .. }
            | EditorAction::SpawnMesh { .. }
            | EditorAction::SpawnBlock { .. }
            | EditorAction::InstantiatePrefab { .. }
    )
}

/// Asks the mirror to catch up on the next frame rather than at the next
/// tick of its cadence.
pub(super) fn pull_soon(resources: &mut Resources) {
    if let Some(sync) = resources.get_mut::<crate::systems::RemoteSyncState>() {
        sync.invalidate();
    }
}

/// Builds one entity on the project out of a captured state.
pub(super) fn build(
    client: &kooch_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    state: &crate::actions::entity_state::EntityState,
    // 🔴 Named at spawn, not set afterwards: parenting preserves the world pose by rewriting the
    // local one, so a `Transform` written first would be divided by the parent's scale.
    parent: Option<kooch_remote::protocol::EntityId>,
    // Which scene to author it into, or `None` to let the captured state
    // restore its own — an undone despawn belongs where it was, a paste
    // belongs where it was asked for.
    scene: Option<kooch_core::Guid>,
) -> Result<kooch_remote::protocol::EntityId, String> {
    let id = client
        .spawn(state.name.as_deref(), scene, parent)
        .map_err(|e| e.to_string())?;
    for component in &state.components {
        if let Err(e) = client.add_component(id, &component.name) {
            tracing::warn!(
                target: "kooch_editor_core::remote_edit::build",
                component = %component.name,
                "the entity did not get a component: {e}",
            );
            continue;
        }
        for (field, value) in &component.fields {
            let value = match to_remote_value(value.clone(), mirror) {
                Ok(value) => value,
                Err(e) => {
                    tracing::debug!(
                        target: "kooch_editor_core::remote_edit::build",
                        component = %component.name,
                        %field,
                        "reference did not translate: {e}",
                    );
                    continue;
                }
            };
            if let Err(e) = client.set_field(id, &component.name, field, value) {
                tracing::debug!(
                    target: "kooch_editor_core::remote_edit::build",
                    component = %component.name,
                    %field,
                    "field did not travel: {e}",
                );
            }
        }
    }
    Ok(id)
}

/// Builds a captured subtree on the project: every entity, then the tree, then the references into
/// it and the prefab membership — all of which name entities that did not exist during the first
/// pass.
pub(super) fn build_tree(
    client: &kooch_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    tree: &crate::actions::entity_state::CapturedTree,
    scene: Option<kooch_core::Guid>,
    into: Option<kooch_remote::protocol::EntityId>,
) -> Result<Vec<kooch_remote::protocol::EntityId>, String> {
    use crate::actions::entity_state;
    use kooch_ecs::reflect::{EntityRef, ReflectValue};

    let mut ids = Vec::with_capacity(tree.len());
    for captured in tree {
        // Only the root is renamed: an author reads a tree by its names.
        let state = match captured.parent {
            None => entity_state::as_copy(&captured.state),
            Some(_) => entity_state::without_identity(&captured.state),
        };
        let parent = captured.parent.map(|parent| ids[parent]).or(into);
        ids.push(build(client, mirror, &state, parent, scene)?);
    }

    for (index, captured) in tree.iter().enumerate() {
        // A reference into the subtree names the copy; one out of it is left alone.
        let inside = |entity: kooch_ecs::entity::Entity| {
            tree.iter()
                .position(|captured| captured.source == entity)
                .map(|at| ids[at])
        };
        for component in &captured.state.components {
            for (field, value) in &component.fields {
                let ReflectValue::EntityRef(Some(reference)) = value else {
                    continue;
                };
                let Some(copy) = reference.entity().and_then(inside) else {
                    continue;
                };
                let value = ReflectValue::EntityRef(Some(EntityRef::live(copy.into())));
                if let Err(e) = client.set_field(ids[index], &component.name, field, value) {
                    tracing::debug!(
                        target: "kooch_editor_core::remote_edit::build_tree",
                        component = %component.name,
                        %field,
                        "a reference into the copy did not travel: {e}",
                    );
                }
            }
        }
    }
    Ok(ids)
}

/// An ECS edit reduced to the fields the remote protocol needs.
enum Edit<'a> {
    SetField {
        entity: kooch_ecs::entity::Entity,
        component: kooch_ecs::component::ComponentId,
        field: &'a str,
        value: &'a kooch_ecs::reflect::ReflectValue,
    },
    AddComponent {
        entity: kooch_ecs::entity::Entity,
        component: kooch_ecs::component::ComponentId,
    },
    RemoveComponent {
        entity: kooch_ecs::entity::Entity,
        component: kooch_ecs::component::ComponentId,
    },
    Despawn(kooch_ecs::entity::Entity),
    /// Reparent, or unparent with `None`.
    Reparent {
        entity: kooch_ecs::entity::Entity,
        new_parent: Option<kooch_ecs::entity::Entity>,
    },
    /// Copy an entity on the project side.
    Duplicate(kooch_ecs::entity::Entity),
    /// Build entities out of the editor's clipboard.
    Paste {
        /// Where the copies land. Named for the same reason
        /// [`Edit::Spawn`]'s is: a paste into a scene somebody
        /// right-clicked must not arrive in the active one.
        into: crate::actions::SpawnTarget,
        states: Vec<crate::actions::entity_state::CapturedTree>,
    },
    /// Re-home an entity: the membership is a component, so this is an
    /// add plus a set rather than a call of its own.
    MoveToScene {
        entity: crate::actions::Entity,
        scene: kooch_core::Guid,
    },
    Spawn {
        name: Option<String>,
        /// Component types the action asked for beyond the base ones.
        extra: Vec<std::any::TypeId>,
        /// Where it goes — the scene, and what it hangs off.
        into: crate::actions::SpawnTarget,
    },
    /// Every field of a `Transform`, from a gizmo drag.
    TransformEdit {
        entity: kooch_ecs::entity::Entity,
        transform: kooch_ecs::transform::Transform,
    },
    /// Write the project's active scene, as `EditorAction::SaveScene`.
    SaveScene {
        as_new: bool,
    },
    /// Move an entity among its siblings on the project.
    MoveEntity {
        entity: kooch_ecs::entity::Entity,
        parent: Option<kooch_ecs::entity::Entity>,
        before: Option<kooch_ecs::entity::Entity>,
    },
    /// Throw away one scene's edits on the project and read it back.
    RevertOneScene(kooch_core::Guid),
    /// Write one named scene of the project's open set to a file.
    SaveOneScene {
        scene: kooch_core::Guid,
        as_new: bool,
    },
    LoadScene {
        /// The file, or `None` to ask for one. The dialog runs on THIS
        /// side either way: the project has no window to put it in.
        path: Option<std::path::PathBuf>,
    },
    /// Open a scene beside what is already loaded, in the project.
    LoadSceneAdditive {
        path: Option<std::path::PathBuf>,
    },
    /// Close one open scene in the project.
    CloseScene(kooch_core::Guid),
    /// Point the project's active slot at an already-open scene.
    SetActiveScene(kooch_core::Guid),
    /// Capture one of the project's entities as a prefab file.
    SavePrefab {
        entity: kooch_ecs::entity::Entity,
        dest: Option<std::path::PathBuf>,
    },
    /// Tell the project a prefab file changed.
    ReloadAssetOnHost(std::path::PathBuf),
    /// Stamp a prefab file into the project's world, optionally placing it.
    InstantiatePrefab {
        path: std::path::PathBuf,
        /// Already resolved to a world position: `classify` runs with the
        /// world available and `dispatch` only has the wire.
        at: Option<glam::Vec3>,
    },
    /// Start or stop the project's gameplay systems in place.
    SetPlaying(bool),
    /// Stop or restart one of the project's systems, from its next
    /// frame.
    SetSystemEnabled {
        name: String,
        nth: u32,
        enabled: bool,
    },
    /// Push a saved prefab's values into every instance the project holds.
    PropagatePrefab(
        Vec<crate::actions::prefab_propagate::PlannedWrite>,
        Vec<crate::actions::prefab_propagate::PlannedRemoval>,
    ),
    /// Drop an instance's overrides and put the prefab's values back.
    RevertToPrefab {
        root: kooch_ecs::entity::Entity,
        overrides: String,
        writes: Vec<crate::actions::prefab_propagate::PlannedWrite>,
    },
}

/// Reduces an action to an [`Edit`], or `None` if remote mode does not own it.
fn classify<'a>(action: &'a EditorAction, resources: &Resources) -> Option<Edit<'a>> {
    match action {
        EditorAction::SetField {
            entity,
            component,
            field,
            value,
        } => Some(Edit::SetField {
            entity: *entity,
            component: *component,
            field,
            value,
        }),
        EditorAction::AddComponent { entity, component } => Some(Edit::AddComponent {
            entity: *entity,
            component: *component,
        }),
        EditorAction::RemoveComponent { entity, component } => Some(Edit::RemoveComponent {
            entity: *entity,
            component: *component,
        }),
        EditorAction::Despawn(entity) => Some(Edit::Despawn(*entity)),
        EditorAction::Reparent { entity, new_parent } => Some(Edit::Reparent {
            entity: *entity,
            new_parent: *new_parent,
        }),
        EditorAction::Duplicate(entity) => Some(Edit::Duplicate(*entity)),
        EditorAction::MoveToScene { entity, scene } => Some(Edit::MoveToScene {
            entity: *entity,
            scene: *scene,
        }),
        // Nothing to send for an empty clipboard, and `None` here would
        // send it down the local path instead of doing nothing.
        EditorAction::PasteEntities { into } => {
            let states = resources
                .get::<crate::clipboard::EntityClipboard>()?
                .states();
            match states.is_empty() {
                true => None,
                false => Some(Edit::Paste {
                    into: *into,
                    states: states.to_vec(),
                }),
            }
        }
        EditorAction::Spawn { name, extra, into } => Some(Edit::Spawn {
            into: *into,
            name: name.clone(),
            extra: extra.clone(),
        }),
        // SpawnMesh is reduced in `dispatch`, not here: resolving its
        // asset mutates the AssetServer, and an `Edit` has to be
        // sendable from an immutable world.
        EditorAction::TransformEdit { entity, after, .. } => Some(Edit::TransformEdit {
            entity: *entity,
            transform: *after,
        }),
        // Scene I/O belongs to the project: the mirror is a view, and
        // saving it locally would write a partly-parked copy over the
        // project's own scene file.
        EditorAction::SaveScene { as_new } => Some(Edit::SaveScene { as_new: *as_new }),
        EditorAction::OpenScene { path } => Some(Edit::LoadScene { path: path.clone() }),
        // Same reason as scene I/O: the world being captured is the project's, and the mirror is a
        // view of it. Writing the mirror would save a partly-parked copy — every component this
        // editor binary has no type for is a name and a bag of fields here.
        EditorAction::SavePrefab { entity, dest, .. } => Some(Edit::SavePrefab {
            entity: *entity,
            dest: dest.clone(),
        }),
        // The guid is resolved to a path here, not sent as one: the wire
        // call names a file on the shared filesystem, and this side is
        // where the asset database that knows the mapping lives.
        EditorAction::InstantiatePrefab { prefab, at } => {
            let path = resources
                .get::<kooch_core::asset_database::AssetDatabase>()
                .and_then(|db| db.entry(*prefab))
                .map(|entry| entry.path.clone())?;
            Some(Edit::InstantiatePrefab {
                path,
                at: crate::viewport_pick::resolve(resources, *at),
            })
        }
        EditorAction::RevertToPrefab { entity, component } => {
            let (root, overrides, writes) =
                crate::actions::prefab_propagate::plan_revert(resources, *entity, *component)?;
            Some(Edit::RevertToPrefab {
                root,
                overrides,
                writes,
            })
        }
        EditorAction::ReloadAssetOnHost(path) => Some(Edit::ReloadAssetOnHost(path.clone())),
        EditorAction::PropagatePrefab(prefab) => {
            let (writes, removals) = crate::actions::prefab_propagate::plan(resources, *prefab);
            Some(Edit::PropagatePrefab(writes, removals))
        }
        // Play runs the project's systems in the project we are already
        // driving, instead of launching a second copy of it.
        EditorAction::SetSystemEnabled { name, nth, enabled } => Some(Edit::SetSystemEnabled {
            name: name.clone(),
            nth: *nth,
            enabled: *enabled,
        }),
        EditorAction::Play => Some(Edit::SetPlaying(true)),
        EditorAction::Stop => Some(Edit::SetPlaying(false)),
        // The wire protocol has one scene, so none of these have anything to send.
        EditorAction::SaveOpenScene(scene) => Some(Edit::SaveOneScene {
            scene: *scene,
            as_new: false,
        }),
        EditorAction::SaveOpenSceneAs(scene) => Some(Edit::SaveOneScene {
            scene: *scene,
            as_new: true,
        }),
        EditorAction::RevertOpenScene(scene) => Some(Edit::RevertOneScene(*scene)),
        EditorAction::MoveEntity {
            entity,
            new_parent,
            before,
        } => Some(Edit::MoveEntity {
            entity: *entity,
            parent: *new_parent,
            before: *before,
        }),
        EditorAction::OpenSceneAdditive { path } => {
            Some(Edit::LoadSceneAdditive { path: path.clone() })
        }
        // 🔴 Both act on the OPEN SET, which is the project's.
        EditorAction::CloseScene(scene) => Some(Edit::CloseScene(*scene)),
        EditorAction::SetActiveScene(scene) => Some(Edit::SetActiveScene(*scene)),
        // Not something remote mode owns (project mgmt, settings, …).
        _ => None,
    }
}

/// The file the caller named, or one asked for now.
fn named_or_asked(
    resources: &Resources,
    path: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    match path {
        Some(path) => Some(path),
        None => crate::actions::scene_io::scene_dialog(resources, None).pick_file(),
    }
}

/// Sends a propagation plan to the project as ordinary protocol calls.
fn push_writes(
    client: &kooch_remote::RemoteClient,
    writes: &[crate::actions::prefab_propagate::PlannedWrite],
    remote: &dyn Fn(kooch_ecs::entity::Entity) -> Result<kooch_remote::protocol::EntityId, String>,
) -> Result<(), String> {
    for write in writes {
        let id = remote(write.entity)?;
        // Before the field, always: a value written into a component that
        // does not exist yet is dropped.
        if write.add_component
            && let Err(e) = client.add_component(id, &write.component)
        {
            tracing::debug!("prefab propagation could not add {}: {e}", write.component);
            continue;
        }
        if write.field.is_empty() {
            continue;
        }
        // A field the project refuses is skipped rather than aborting the
        // rest: one stale component must not stop the other instances from
        // catching up.
        if let Err(e) = client.set_field(id, &write.component, &write.field, write.value.clone()) {
            tracing::debug!(
                "prefab propagation skipped {}.{}: {e}",
                write.component,
                write.field,
            );
        }
    }
    Ok(())
}

/// Translates a value's entity references from mirror handles to the ones the project uses.
pub(super) fn to_remote_value(
    value: kooch_ecs::reflect::ReflectValue,
    mirror: &crate::remote_mirror::RemoteMirror,
) -> Result<kooch_ecs::reflect::ReflectValue, String> {
    use kooch_ecs::reflect::{EntityRef, ReflectValue};

    let ReflectValue::EntityRef(Some(reference)) = value else {
        return Ok(value);
    };
    // A persistent reference names an identity, not a handle, and means
    // the same thing in both processes.
    let Some(local) = reference.entity() else {
        return Ok(ReflectValue::EntityRef(Some(reference)));
    };
    let remote = mirror
        .remote_of(local)
        .ok_or_else(|| "the referenced entity is not in the mirror".to_owned())?;
    Ok(ReflectValue::EntityRef(Some(EntityRef::live(
        remote.into(),
    ))))
}

mod send;
mod spawn;

use send::*;
use spawn::*;

#[cfg(test)]
mod tests;
