//! The remote sink of the dual-sink edit dispatch.
//!
//! In local mode an [`EditorAction`] mutates the editor's own ECS. In
//! remote mode the ECS is a mirror of a project that owns the real state,
//! so the same action is routed over the wire instead — the mirror then
//! catches up on the next refresh. This is the *only* place the two modes
//! diverge: panels, DTOs and the viewport are identical either way.
//!
//! Translation resolves the editor's local identities back to the
//! project's: a mirrored [`Entity`] to its remote
//! [`EntityId`](kooch_remote::protocol::EntityId) via the mirror, and a
//! [`ComponentId`] to its type name via the interner — the same name the
//! server keys components by.

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentNames;

use crate::actions::EditorAction;
use crate::remote_session::RemoteState;

/// Attempts to handle `action` over the wire.
///
/// Returns `true` when the action is an ECS edit that was routed to the
/// server (or dropped because it cannot be, e.g. an unresolved entity) —
/// the caller must not also apply it locally. Returns `false` for actions
/// that remote mode does not own (project management, editor settings),
/// which the caller handles through the normal local path.
///
/// The caller guarantees a connected session before calling.
pub(crate) fn dispatch(resources: &mut Resources, action: &EditorAction) -> bool {
    // Undo/Redo travel as inverses of the edits already sent — the local
    // command stack describes the mirror, which the next refresh
    // overwrites. See [`crate::actions::remote_undo`].
    if matches!(action, EditorAction::Undo | EditorAction::Redo) {
        crate::actions::remote_undo::step(resources, matches!(action, EditorAction::Undo));
        return true;
    }

    // Spawning a mesh is the one edit that cannot be reduced to a single
    // protocol call: the editor has to load the asset to learn its GUID,
    // and loading mutates the `AssetServer`, which `send` cannot do from
    // an immutable world. Handled here, before `classify`.
    if let EditorAction::SpawnMesh { path, name } = action {
        spawn_mesh(resources, path, name);
        return true;
    }

    // Non-ECS actions stay on the local path even in remote mode: closing
    // the project, toggling power profiles all act on the editor, not the
    // remote world.
    let Some(edit) = classify(action, resources) else {
        return false;
    };

    // Lifted out of Resources so the send can borrow the session and the
    // rest of the world at the same time, and record play state after.
    let Some(mut state) = resources.remove::<RemoteState>() else {
        return true;
    };
    let playing = matches!(edit, Edit::SetPlaying(playing) if playing);
    let is_play_toggle = matches!(edit, Edit::SetPlaying(_));

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

    resources.insert(state);

    if sent {
        crate::actions::remote_undo::record(resources, action, before, created);
    }

    // The project wrote the file; this side has to be told it exists, or
    // the Inspector cannot find what the user just made until the editor
    // restarts. Done here rather than in `send`, which holds the world
    // immutably so it can borrow the session alongside it.
    if let Some(path) = saved_prefab.filter(|_| sent) {
        crate::actions::handlers::asset_saved(resources, &path);
        // The project wrote bytes this side's cache has never seen.
        crate::actions::handlers::prefab_saved(resources, &path);
    }
    true
}

/// Builds a mesh-bound entity on the project's side.
///
/// The editor resolves the asset locally — both processes see the same
/// filesystem, so the GUID it gets is the GUID the project will resolve —
/// then assembles the entity out of calls the protocol already has:
/// `spawn` for the entity and its `Name`, `add_component` for `Transform`
/// and `MeshRenderer`, and `set_field` to write the mesh reference.
/// `MeshRenderer.mesh` is reflected as a typed `AssetRef`, so it goes over
/// the wire like any other field.
///
/// Failures are logged with the path that caused them. The bug this
/// replaces was the silence: a menu entry that did nothing at all.
fn spawn_mesh(resources: &mut Resources, path: &std::path::Path, name: &str) {
    const TARGET: &str = "kooch_editor_core::remote_edit::spawn_mesh";

    let Some((guid, asset_type)) = resolve_mesh_asset(resources, path) else {
        return;
    };

    let Some(state) = resources.get::<RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let client = session.client();

    let entity = match client.spawn(Some(name)) {
        Ok(entity) => entity,
        Err(e) => {
            tracing::warn!(target: TARGET, error = %e, "remote spawn failed");
            return;
        }
    };

    // Remote `spawn` creates only `Name`, so both of these are needed.
    let transform_ty = std::any::type_name::<kooch_ecs::transform::Transform>();
    let renderer_ty = std::any::type_name::<kooch_ecs::mesh_renderer::MeshRenderer>();
    for ty in [transform_ty, renderer_ty] {
        if let Err(e) = client.add_component(entity, ty) {
            tracing::warn!(target: TARGET, component = ty, error = %e, "add_component failed");
            return;
        }
    }

    let value = kooch_ecs::reflect::ReflectValue::AssetRef {
        guid: Some(guid),
        asset_type,
    };
    if let Err(e) = client.set_field(entity, renderer_ty, "mesh", value) {
        tracing::warn!(target: TARGET, error = %e, "could not write the mesh reference");
        return;
    }

    tracing::info!(
        target: TARGET,
        %name,
        path = %path.display(),
        %guid,
        "spawned a mesh entity on the project",
    );
}

/// Loads a mesh asset locally and returns its GUID and asset type name.
fn resolve_mesh_asset(
    resources: &mut Resources,
    path: &std::path::Path,
) -> Option<(kooch_core::Guid, String)> {
    const TARGET: &str = "kooch_editor_core::remote_edit::spawn_mesh";
    use kooch_core::asset_database::AssetDatabase;
    use kooch_core::asset_loader::AssetServer;
    use kooch_render::meshlet::MeshletMesh;

    // Taken out so the load can borrow the server and the world at once,
    // the same dance `SpawnMeshCommand` does on the local path.
    let mut server = match resources.remove::<AssetServer>() {
        Some(server) => server,
        None => {
            tracing::warn!(target: TARGET, "AssetServer missing; cannot resolve the mesh");
            return None;
        }
    };
    let loaded = server.load::<MeshletMesh>(path, resources);
    let resolved = server.resolve_path(path);
    resources.insert(server);

    if let Err(e) = loaded {
        tracing::warn!(
            target: TARGET,
            path = %path.display(),
            error = %e,
            "failed to load the mesh asset",
        );
        return None;
    }

    let guid = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.guid_for(&resolved));
    if guid.is_none() {
        tracing::warn!(
            target: TARGET,
            resolved = %resolved.display(),
            "the AssetDatabase has no entry for the loaded asset",
        );
    }
    guid.map(|guid| (guid, std::any::type_name::<MeshletMesh>().to_owned()))
}

/// Copies an entity on the project side, out of what the mirror already
/// knows.
///
/// The editor holds every reflected component and its current values for
/// the mirrored source, so no protocol method is needed: it decomposes
/// into spawn + `add_component` + `set_field`, which is what [`build`]
/// does for every entity the editor creates remotely.
fn duplicate(
    entity: kooch_ecs::entity::Entity,
    client: &kooch_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    resources: &Resources,
) -> Result<kooch_remote::protocol::EntityId, String> {
    // Only to fail early with a clear reason: an entity the mirror does
    // not know is one the project never had.
    mirror
        .remote_of(entity)
        .ok_or_else(|| "entity not in mirror".to_owned())?;

    let state = crate::actions::entity_state::as_copy(&crate::actions::entity_state::capture(
        resources, entity,
    ));
    let copy = build(client, mirror, &state)?;
    tracing::info!(
        target: "kooch_editor_core::remote_edit::duplicate",
        components = state.components.len(),
        "duplicated an entity on the project",
    );
    Ok(copy)
}

/// Builds one entity on the project out of a captured state.
///
/// The one place an entity is created remotely from values: duplicate,
/// paste and undoing a despawn all land here, so a component that fails
/// to travel fails the same way for all three.
///
/// Fields that fail to apply are logged and skipped rather than aborting.
/// A half-copied entity the user can see and fix beats an entity that was
/// never created because one opaque field would not travel.
pub(super) fn build(
    client: &kooch_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    state: &crate::actions::entity_state::EntityState,
) -> Result<kooch_remote::protocol::EntityId, String> {
    let id = client
        .spawn(state.name.as_deref())
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
    ///
    /// Its own protocol method rather than a `SetField` on `Parent`, whose
    /// `reflect_set` is read-only: an entity handle is not a reflectable
    /// value. Before this the action fell through to the local path, mutated
    /// the *mirror*, and silently reverted on the next refresh (#595).
    Reparent {
        entity: kooch_ecs::entity::Entity,
        new_parent: Option<kooch_ecs::entity::Entity>,
    },
    /// Copy an entity on the project side.
    ///
    /// No protocol method needed: the editor already holds every component's
    /// values for the source entity in the mirror, so this decomposes into
    /// spawn + add_component + set_field. Before this it was claimed by
    /// nobody at all and dropped in silence (#595).
    Duplicate(kooch_ecs::entity::Entity),
    /// Build entities out of the editor's clipboard.
    ///
    /// Carries the values rather than the source entities: what was
    /// copied is a value, and it still pastes after the entity it came
    /// from has been deleted — or after the project it came from was
    /// restarted.
    ///
    /// Owned, unlike its neighbours: the values come from the clipboard
    /// resource, and the borrow of `Resources` that reads it ends before
    /// the send that needs the session out of the same `Resources`.
    Paste(Vec<crate::actions::entity_state::EntityState>),
    Spawn {
        name: Option<String>,
        /// Component types the action asked for beyond the base ones.
        ///
        /// Dropped before this existed, which is why a light spawned
        /// remotely arrived with a `Name` and nothing else — no
        /// `Transform`, no light component.
        extra: Vec<std::any::TypeId>,
    },
    /// Every field of a `Transform`, from a gizmo drag.
    TransformEdit {
        entity: kooch_ecs::entity::Entity,
        transform: kooch_ecs::transform::Transform,
    },
    /// Write the project's world to a scene file, or replace it from one.
    SaveScene,
    LoadScene,
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
    /// Push a saved prefab's values into every instance the project holds.
    ///
    /// Carries the writes rather than a guid: working out *which* fields
    /// go where needs the mirror, the prefab's cached document and each
    /// instance's override set, all of which live on this side.
    PropagatePrefab(
        Vec<crate::actions::prefab_propagate::PlannedWrite>,
        Vec<crate::actions::prefab_propagate::PlannedRemoval>,
    ),
    /// Drop an instance's overrides and put the prefab's values back.
    ///
    /// The new override set travels with the writes: applying one without
    /// the other leaves the instance either showing the user's numbers
    /// while claiming to be clean, or clean until the next propagation
    /// puts them back.
    RevertToPrefab {
        root: kooch_ecs::entity::Entity,
        overrides: String,
        writes: Vec<crate::actions::prefab_propagate::PlannedWrite>,
    },
}

/// Reduces an action to an [`Edit`], or `None` if remote mode does not
/// own it.
///
/// Takes the world because a couple of actions cannot be reduced without
/// reading it: a viewport drop names a place on screen, and the camera that
/// turns it into a world position lives here. `dispatch` is past that point
/// — it has the wire and nothing else.
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
        // Nothing to send for an empty clipboard, and `None` here would
        // send it down the local path instead of doing nothing.
        EditorAction::PasteEntities => {
            let states = resources
                .get::<crate::clipboard::EntityClipboard>()?
                .states();
            match states.is_empty() {
                true => None,
                false => Some(Edit::Paste(states.to_vec())),
            }
        }
        EditorAction::Spawn { name, extra } => Some(Edit::Spawn {
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
        EditorAction::SaveScene => Some(Edit::SaveScene),
        EditorAction::OpenScene => Some(Edit::LoadScene),
        // Same reason as scene I/O: the world being captured is the
        // project's, and the mirror is a view of it. Writing the mirror
        // would save a partly-parked copy — every component this editor
        // binary has no type for is a name and a bag of fields here.
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
        EditorAction::Play => Some(Edit::SetPlaying(true)),
        EditorAction::Stop => Some(Edit::SetPlaying(false)),
        // The wire protocol has one scene, so none of these have anything
        // to send. Additive loading is refused outright while mirroring
        // rather than handled here: entities loaded on this side do not
        // exist in the project, so they are invisible in the game and
        // every edit to them is dropped for not being in the mirror.
        // Listed rather than left to the catch-all so the audit in #596
        // keeps meaning something.
        EditorAction::OpenSceneAdditive
        | EditorAction::CloseScene(_)
        | EditorAction::SetActiveScene(_) => None,
        // Not something remote mode owns (project mgmt, settings, …).
        _ => None,
    }
}

/// Sends a propagation plan to the project as ordinary protocol calls.
///
/// Not as `EditorAction`s: an edit on an instance is recorded as an
/// override, so routing propagation through the action layer would pin
/// every field it touched and the instance would stop following the
/// prefab. A protocol call has no such side effect.
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

/// Translates a value's entity references from mirror handles to the ones
/// the project uses.
///
/// The mirror's entities are the editor's own; the project has its own
/// handles for the same entities, and every method that names an entity
/// goes through `remote_of` for exactly this reason. A reference *inside*
/// a value needs it too — sent as-is, the picker would point a joint at
/// whatever the project happens to have at that index.
///
/// Anything else passes through untouched.
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

/// Sends one [`Edit`] to the project's server.
///
/// `created` collects the ids of entities the edit brought into being.
/// An out-parameter rather than a return value because only four of the
/// twenty arms create anything, and threading `Vec::new()` through the
/// other sixteen would say nothing sixteen times.
fn send(
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
        Edit::Paste(states) => {
            for state in &states {
                created.push(build(
                    &client,
                    mirror,
                    &crate::actions::entity_state::as_copy(state),
                )?);
            }
            Ok(())
        }
        Edit::Spawn {
            name: entity_name,
            extra,
        } => {
            let entity = client.spawn(entity_name.as_deref()).map_err(map_err)?;
            created.push(entity);
            // Remote `spawn` creates only `Name`, while the local path adds
            // Name + Transform + extras. Everything past the name has to be
            // asked for explicitly, or the entity arrives inert — a light
            // with no Transform has no position and no direction.
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
        // Both processes see the same filesystem, so the path the user
        // picks here is meaningful on the project's side of the wire.
        Edit::SaveScene => match crate::actions::scene_io::scene_dialog(resources).save_file() {
            Some(path) => client.save_scene(&path.to_string_lossy()).map_err(map_err),
            None => Ok(()),
        },
        Edit::LoadScene => match crate::actions::scene_io::scene_dialog(resources).pick_file() {
            Some(path) => client.load_scene(&path.to_string_lossy()).map_err(map_err),
            None => Ok(()),
        },
        Edit::SetPlaying(playing) => client.set_playing(playing).map_err(map_err),
        // Sent as ordinary field writes, but *not* as `EditorAction`s: an
        // edit on an instance is recorded as an override, so routing
        // propagation through the action layer would pin every field it
        // touched and the instance would stop following the prefab. The
        // protocol call has no such side effect.
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
            // Placing the instance is a `SetField` on the root that just
            // came back, rather than a parameter on the call. It reuses the
            // path that already knows how to write a reflected field, and
            // keeps spatial types out of the wire format.
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

#[cfg(test)]
mod tests;
