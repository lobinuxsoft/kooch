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
//! [`EntityId`](ome_remote::protocol::EntityId) via the mirror, and a
//! [`ComponentId`] to its type name via the interner — the same name the
//! server keys components by.

use ome_core::resource::Resources;
use ome_ecs::component::ComponentNames;

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
    // Undo/Redo have no remote form yet; swallow them in remote mode
    // rather than replaying against the mirror (which the next refresh
    // overwrites anyway).
    if matches!(action, EditorAction::Undo | EditorAction::Redo) {
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

    let mut sent = false;
    if let Some(session) = state.session.as_ref() {
        let names = resources.get::<ComponentNames>();
        match send(edit, session, &state.mirror, names, resources) {
            Ok(()) if is_play_toggle => {
                state.playing = playing;
                sent = true;
            }
            Ok(()) => sent = true,
            Err(e) => tracing::warn!("remote edit dropped: {e}"),
        }
    }

    resources.insert(state);

    // The project wrote the file; this side has to be told it exists, or
    // the Inspector cannot find what the user just made until the editor
    // restarts. Done here rather than in `send`, which holds the world
    // immutably so it can borrow the session alongside it.
    if let Some(path) = saved_prefab.filter(|_| sent) {
        crate::actions::handlers::register_saved_asset(resources, &path);
        // The project wrote bytes this side's cache has never seen.
        crate::actions::handlers::refresh_cached_prefab(resources, &path);
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
    const TARGET: &str = "ome_editor_core::remote_edit::spawn_mesh";

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
    let transform_ty = std::any::type_name::<ome_ecs::transform::Transform>();
    let renderer_ty = std::any::type_name::<ome_ecs::mesh_renderer::MeshRenderer>();
    for ty in [transform_ty, renderer_ty] {
        if let Err(e) = client.add_component(entity, ty) {
            tracing::warn!(target: TARGET, component = ty, error = %e, "add_component failed");
            return;
        }
    }

    let value = ome_ecs::reflect::ReflectValue::AssetRef {
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
) -> Option<(ome_core::Guid, String)> {
    const TARGET: &str = "ome_editor_core::remote_edit::spawn_mesh";
    use ome_core::asset_database::AssetDatabase;
    use ome_core::asset_loader::AssetServer;
    use ome_render::meshlet::MeshletMesh;

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
/// the mirrored source, so no protocol method is needed: spawn, then
/// `add_component` per component, then `set_field` per field. The same
/// decomposition `SpawnMesh` uses.
///
/// Fields that fail to apply are logged and skipped rather than aborting.
/// A half-copied entity the user can see and fix beats an entity that was
/// never created because one opaque field would not travel.
fn duplicate(
    entity: ome_ecs::entity::Entity,
    client: &ome_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    resources: &Resources,
) -> Result<(), String> {
    use ome_ecs::component::ComponentRegistry;

    let source = mirror
        .remote_of(entity)
        .ok_or_else(|| "entity not in mirror".to_owned())?;
    let registry = resources
        .get::<ComponentRegistry>()
        .ok_or_else(|| "no component registry".to_owned())?;

    // Read the source's components off the *mirror*, which is the editor's
    // copy of what the project has.
    let components: Vec<(String, Vec<(String, ome_ecs::reflect::ReflectValue)>)> = registry
        .reflected_type_names()
        .into_iter()
        .filter_map(|(type_id, name)| {
            let fields = registry.reflect_get_fields(&type_id, entity)?;
            Some((name.to_owned(), fields))
        })
        .collect();

    let name = components
        .iter()
        .find(|(n, _)| n.ends_with("::Name"))
        .and_then(|(_, fields)| fields.iter().find(|(f, _)| f == "value"))
        .and_then(|(_, v)| match v {
            ome_ecs::reflect::ReflectValue::String(s) => Some(format!("{s} Copy")),
            _ => None,
        });

    let copy = client.spawn(name.as_deref()).map_err(|e| e.to_string())?;

    for (type_name, fields) in &components {
        if let Err(e) = client.add_component(copy, type_name) {
            tracing::warn!(
                target: "ome_editor_core::remote_edit::duplicate",
                component = %type_name,
                error = %e,
                "could not add a component to the copy",
            );
            continue;
        }
        for (field, value) in fields {
            if let Err(e) = client.set_field(copy, type_name, field, value.clone()) {
                tracing::debug!(
                    target: "ome_editor_core::remote_edit::duplicate",
                    component = %type_name,
                    %field,
                    error = %e,
                    "field did not copy",
                );
            }
        }
    }

    tracing::info!(
        target: "ome_editor_core::remote_edit::duplicate",
        ?source,
        components = components.len(),
        "duplicated an entity on the project",
    );
    Ok(())
}

/// An ECS edit reduced to the fields the remote protocol needs.
enum Edit<'a> {
    SetField {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
        field: &'a str,
        value: &'a ome_ecs::reflect::ReflectValue,
    },
    AddComponent {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
    },
    RemoveComponent {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
    },
    Despawn(ome_ecs::entity::Entity),
    /// Reparent, or unparent with `None`.
    ///
    /// Its own protocol method rather than a `SetField` on `Parent`, whose
    /// `reflect_set` is read-only: an entity handle is not a reflectable
    /// value. Before this the action fell through to the local path, mutated
    /// the *mirror*, and silently reverted on the next refresh (#595).
    Reparent {
        entity: ome_ecs::entity::Entity,
        new_parent: Option<ome_ecs::entity::Entity>,
    },
    /// Copy an entity on the project side.
    ///
    /// No protocol method needed: the editor already holds every component's
    /// values for the source entity in the mirror, so this decomposes into
    /// spawn + add_component + set_field. Before this it was claimed by
    /// nobody at all and dropped in silence (#595).
    Duplicate(ome_ecs::entity::Entity),
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
        entity: ome_ecs::entity::Entity,
        transform: ome_ecs::transform::Transform,
    },
    /// Write the project's world to a scene file, or replace it from one.
    SaveScene,
    LoadScene,
    /// Capture one of the project's entities as a prefab file.
    SavePrefab {
        entity: ome_ecs::entity::Entity,
        dest: Option<std::path::PathBuf>,
    },
    /// Tell the project a prefab file changed.
    ReloadPrefabOnHost(std::path::PathBuf),
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
        root: ome_ecs::entity::Entity,
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
                .get::<ome_core::asset_database::AssetDatabase>()
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
        EditorAction::ReloadPrefabOnHost(path) => Some(Edit::ReloadPrefabOnHost(path.clone())),
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
    client: &ome_remote::RemoteClient,
    writes: &[crate::actions::prefab_propagate::PlannedWrite],
    remote: &dyn Fn(ome_ecs::entity::Entity) -> Result<ome_remote::protocol::EntityId, String>,
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
fn to_remote_value(
    value: ome_ecs::reflect::ReflectValue,
    mirror: &crate::remote_mirror::RemoteMirror,
) -> Result<ome_ecs::reflect::ReflectValue, String> {
    use ome_ecs::reflect::{EntityRef, ReflectValue};

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
fn send(
    edit: Edit<'_>,
    session: &crate::remote_session::RemoteSession,
    mirror: &crate::remote_mirror::RemoteMirror,
    names: Option<&ComponentNames>,
    resources: &Resources,
) -> Result<(), String> {
    use ome_ecs::reflect::ReflectValue;

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
    let map_err = |e: ome_remote::ClientError| e.to_string();

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
        Edit::Duplicate(entity) => duplicate(entity, &client, mirror, resources),
        Edit::Spawn {
            name: entity_name,
            extra,
        } => {
            let entity = client.spawn(entity_name.as_deref()).map_err(map_err)?;
            // Remote `spawn` creates only `Name`, while the local path adds
            // Name + Transform + extras. Everything past the name has to be
            // asked for explicitly, or the entity arrives inert — a light
            // with no Transform has no position and no direction.
            let transform = std::any::type_name::<ome_ecs::transform::Transform>();
            client.add_component(entity, transform).map_err(map_err)?;

            // `extra` is a list of local `TypeId`s; the server keys
            // components by name, and the registry is the only thing that
            // knows the mapping.
            let registry = resources.get::<ome_ecs::component::ComponentRegistry>();
            for type_id in extra {
                let Some(component_name) =
                    registry.as_ref().and_then(|r| r.component_name(&type_id))
                else {
                    // Nothing to send and nothing to guess: a type the
                    // local registry has never seen has no name the server
                    // would recognise.
                    tracing::warn!(
                        target: "ome_editor_core::remote_edit",
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
            let ty = std::any::type_name::<ome_ecs::transform::Transform>();
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
                    std::any::type_name::<ome_ecs::prefab_instance::PrefabInstance>(),
                    "overrides",
                    ReflectValue::String(overrides),
                )
                .map_err(map_err)
        }
        Edit::ReloadPrefabOnHost(path) => client
            .reload_prefab(&path.to_string_lossy())
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
                    std::any::type_name::<ome_ecs::transform::Transform>(),
                    "position",
                    ReflectValue::Vec3(at),
                )
                .map_err(map_err)
        }
    }
}

#[cfg(test)]
mod tests {
    /// A socket name unique to this test.
    ///
    /// Tests run in parallel in one process, so a shared name would have
    /// them binding over each other — the local-socket equivalent of the
    /// port scan this replaced, but solved instead of retried.
    fn test_socket_name() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static N: AtomicU32 = AtomicU32::new(0);
        // The counter alone is not enough: it is per-module, so two test
        // modules in one binary both start at zero and collide on the same
        // name. The clock disambiguates without the modules having to know
        // about each other.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        format!(
            "ome_test_{}_{}_{}.sock",
            std::process::id(),
            nanos,
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    use std::any::TypeId;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ome_core::resource::Resources;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::commands::Commands;
    use ome_ecs::component::{ComponentNames, ComponentRegistry};
    use ome_ecs::dynamic_components::DynamicComponents;
    use ome_ecs::name::Name;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::ReflectValue;
    use ome_ecs::transform::Transform;

    use ome_remote::RemoteClient;
    use ome_remote::handlers::handle;
    use ome_remote::protocol::{Method, Request};
    use ome_remote::server::RemoteServer;

    use super::*;
    use crate::remote_session::{ConnectionState, RemoteSession, RemoteState};

    fn ecs() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        {
            let reg = r.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Name>();
            reg.register_cpu_reflected::<Transform>();
        }
        r
    }

    /// A `SetField` issued in the editor lands on the project's server.
    #[test]
    fn set_field_routes_to_the_server() {
        let transform_ty = std::any::type_name::<Transform>();

        // Server side: a project with one Transform-bearing entity.
        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);
        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            let hero = match handle(
                &Request {
                    id: 0,
                    method: Method::Spawn {
                        name: Some("Hero".into()),
                    },
                },
                &mut res,
            )
            .payload
            {
                ome_remote::protocol::ResponsePayload::Result(
                    ome_remote::protocol::ResponseData::Spawned { entity },
                ) => entity,
                _ => panic!("spawn"),
            };
            handle(
                &Request {
                    id: 1,
                    method: Method::AddComponent {
                        entity: hero,
                        component: transform_ty.into(),
                    },
                },
                &mut res,
            );
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        // Editor side: connect, mirror, then issue an edit.
        let mut editor = ecs();
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");

        let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
        state.mirror.apply(&snapshot, &mut editor);
        let remote_id = snapshot[0].id;
        let local = state.mirror.local_of(remote_id).expect("mirrored");
        let comp = editor
            .get_mut::<ComponentNames>()
            .unwrap()
            .intern(transform_ty);
        editor.insert(state);
        let mut editor = editor;

        let action = EditorAction::SetField {
            entity: local,
            component: comp,
            field: "position".into(),
            value: ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)),
        };
        assert!(
            dispatch(&mut editor, &action),
            "remote dispatch should own SetField"
        );

        // The server now holds the edited value.
        let client = RemoteClient::new(&socket);
        let entities = client.list_entities().unwrap();
        let pos = entities[0]
            .components
            .iter()
            .find(|c| c.type_name.ends_with("Transform"))
            .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
            .map(|(_, v)| v.clone());
        assert_eq!(
            pos,
            Some(ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)))
        );

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }

    /// Play is a wire toggle in remote mode: the project runs its own
    /// systems in place, and the editor records that it is playing so
    /// the toolbar and the refresh cadence follow.
    #[test]
    fn play_toggles_the_remote_gate() {
        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);
        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            ome_core::run_state::Playing::is_playing(&res)
        });

        let mut editor = ecs();
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");
        assert!(!state.playing, "starts paused");
        editor.insert(state);

        assert!(dispatch(&mut editor, &EditorAction::Play));
        assert!(
            editor.get::<RemoteState>().unwrap().playing,
            "editor did not record the play state"
        );

        done.store(true, Ordering::Relaxed);
        assert!(main_loop.join().unwrap(), "project did not start playing");
    }

    /// A non-ECS action is not owned by the remote sink.
    #[test]
    fn non_ecs_action_falls_through() {
        let mut editor = ecs();
        assert!(!dispatch(&mut editor, &EditorAction::CloseProject));
    }

    /// The engine root, two levels above this crate's manifest.
    fn engine_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("ome_editor_core is not two levels below the engine root")
            .to_path_buf()
    }

    /// An editor world that can resolve mesh assets out of `assets/`.
    fn editor_with_assets() -> Resources {
        use ome_core::asset_database::AssetDatabase;
        use ome_core::asset_loader::AssetServer;
        use ome_ecs::mesh_renderer::MeshRenderer;

        let mut r = ecs();
        {
            let reg = r.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<MeshRenderer>();
        }
        let mut server = AssetServer::new().with_asset_root(engine_root().join("assets"));
        server.register_loader::<ome_render::meshlet::MeshletMesh, _>(
            ome_render::meshlet::MeshletMeshLoader,
        );
        r.insert(server);
        r.insert(AssetDatabase::new());
        r.insert(ome_core::assets::Assets::<ome_render::meshlet::MeshletMesh>::new());
        r
    }

    /// The bug: `Spawn ▸ 3D Object` was dropped on the floor in remote
    /// mode, because `classify` had no arm for it and nothing else claimed
    /// it. Silently — no entity, no error.
    ///
    /// The project has to end up with an entity carrying `Name`,
    /// `Transform` and `MeshRenderer`, with the mesh reference written.
    #[test]
    fn spawn_mesh_builds_the_entity_on_the_project() {
        use ome_ecs::mesh_renderer::MeshRenderer;

        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);

        // The project: a world that can hold a MeshRenderer.
        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            {
                let reg = res.get_mut::<ComponentRegistry>().unwrap();
                reg.register_cpu_reflected::<MeshRenderer>();
            }
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let mut editor = editor_with_assets();
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");
        editor.insert(state);

        let action = EditorAction::SpawnMesh {
            path: std::path::PathBuf::from("meshes/suzanne.glb"),
            name: "Suzanne".to_owned(),
        };
        assert!(
            dispatch(&mut editor, &action),
            "the remote sink still does not own SpawnMesh"
        );

        // Ask the project what it has.
        let client = RemoteClient::new(&socket);
        let entities = client.list_entities().unwrap();
        let spawned = entities
            .iter()
            .find(|e| {
                e.components
                    .iter()
                    .any(|c| c.type_name.ends_with("MeshRenderer"))
            })
            .expect("the project has no entity with a MeshRenderer");

        for expected in ["Name", "Transform", "MeshRenderer"] {
            assert!(
                spawned
                    .components
                    .iter()
                    .any(|c| c.type_name.ends_with(expected)),
                "the spawned entity has no {expected}: {:?}",
                spawned
                    .components
                    .iter()
                    .map(|c| &c.type_name)
                    .collect::<Vec<_>>()
            );
        }

        let mesh = spawned
            .components
            .iter()
            .find(|c| c.type_name.ends_with("MeshRenderer"))
            .and_then(|c| c.fields.iter().find(|(n, _)| n == "mesh"))
            .map(|(_, v)| v.clone())
            .expect("MeshRenderer has no mesh field");
        assert!(
            matches!(mesh, ReflectValue::AssetRef { guid: Some(_), .. }),
            "the mesh reference did not reach the project: {mesh:?}"
        );

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }

    /// An unresolvable path is claimed and logged, not passed through to
    /// the local path where it would spawn into the mirror — which the
    /// next refresh would wipe, looking like a flicker.
    #[test]
    fn an_unresolvable_mesh_is_still_owned_by_the_remote_sink() {
        let mut editor = editor_with_assets();
        editor.insert(RemoteState::new());

        let action = EditorAction::SpawnMesh {
            path: std::path::PathBuf::from("meshes/does_not_exist.glb"),
            name: "Ghost".to_owned(),
        };
        assert!(dispatch(&mut editor, &action));
    }

    /// What was reported: a light spawned remotely arrived with a `Name`
    /// and nothing else. `classify` matched `EditorAction::Spawn { name, .. }`
    /// and the `..` threw away the component list, while remote `spawn`
    /// creates only `Name` — so no Transform, no light component, an entity
    /// with no position and nothing to render.
    #[test]
    fn spawn_carries_its_extra_components_over_the_wire() {
        use ome_ecs::directional_light::DirectionalLight;

        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);

        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            {
                let reg = res.get_mut::<ComponentRegistry>().unwrap();
                reg.register_cpu_reflected::<DirectionalLight>();
            }
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let mut editor = ecs();
        {
            let reg = editor.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<DirectionalLight>();
        }
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");
        editor.insert(state);

        let action = EditorAction::Spawn {
            extra: vec![std::any::TypeId::of::<DirectionalLight>()],
            name: Some("Directional Light".to_owned()),
        };
        assert!(dispatch(&mut editor, &action));

        let client = RemoteClient::new(&socket);
        let entities = client.list_entities().unwrap();
        let light = entities
            .iter()
            .find(|e| {
                e.components
                    .iter()
                    .any(|c| c.type_name.ends_with("DirectionalLight"))
            })
            .unwrap_or_else(|| {
                panic!(
                    "the light component never reached the project; got {:?}",
                    entities
                        .iter()
                        .map(|e| e
                            .components
                            .iter()
                            .map(|c| &c.type_name)
                            .collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                )
            });

        // A light with no Transform has no position and no direction, so
        // this is not a nice-to-have — it is the difference between a light
        // and an inert entity.
        for expected in ["Name", "Transform", "DirectionalLight"] {
            assert!(
                light
                    .components
                    .iter()
                    .any(|c| c.type_name.ends_with(expected)),
                "the spawned light has no {expected}: {:?}",
                light
                    .components
                    .iter()
                    .map(|c| &c.type_name)
                    .collect::<Vec<_>>()
            );
        }

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }

    /// #595, hole one: `Reparent` had no `classify` arm, fell through to the
    /// local path, mutated the *mirror*, and silently reverted half a second
    /// later when the refresh rebuilt parent links from the project.
    #[test]
    fn reparent_reaches_the_project() {
        use ome_ecs::hierarchy::Parent;

        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);

        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            {
                let reg = res.get_mut::<ComponentRegistry>().unwrap();
                reg.register_cpu_reflected::<Parent>();
                reg.register_cpu_reflected::<ome_ecs::hierarchy::Children>();
            }
            // Two root entities for the editor to relate.
            for name in ["Parent", "Child"] {
                handle(
                    &Request {
                        id: 1,
                        method: Method::Spawn {
                            name: Some(name.to_owned()),
                        },
                    },
                    &mut res,
                );
            }
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let mut editor = ecs();
        {
            let reg = editor.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Parent>();
            reg.register_cpu_reflected::<ome_ecs::hierarchy::Children>();
        }
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");

        let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
        state.mirror.apply(&snapshot, &mut editor);
        assert!(snapshot.len() >= 2, "the project has no pair to relate");
        let parent = state.mirror.local_of(snapshot[0].id).expect("mirrored");
        let child = state.mirror.local_of(snapshot[1].id).expect("mirrored");
        editor.insert(state);

        assert!(dispatch(
            &mut editor,
            &EditorAction::Reparent {
                entity: child,
                new_parent: Some(parent),
            }
        ));

        // The project itself has to report the relationship — not the mirror,
        // which is exactly what used to be mutated instead.
        let client = RemoteClient::new(&socket);
        let entities = client.list_entities().unwrap();
        let child_remote = entities
            .iter()
            .find(|e| e.id == snapshot[1].id)
            .expect("child gone");
        assert_eq!(
            child_remote.parent,
            Some(snapshot[0].id),
            "the reparent never reached the project"
        );

        // And unparenting travels the same way.
        assert!(dispatch(
            &mut editor,
            &EditorAction::Reparent {
                entity: child,
                new_parent: None,
            }
        ));
        let entities = client.list_entities().unwrap();
        let child_remote = entities.iter().find(|e| e.id == snapshot[1].id).unwrap();
        assert_eq!(child_remote.parent, None, "unparenting did not travel");

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }

    /// #595, hole two: `Duplicate` was claimed by nothing at all — not
    /// `classify`, not `dispatch`, not `apply_non_ecs_action` — so in remote
    /// mode it was a silent no-op.
    #[test]
    fn duplicate_creates_a_copy_on_the_project() {
        let server = RemoteServer::start(&test_socket_name()).expect("bind");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);

        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            let entity = {
                let mut commands = res.remove::<Commands>().unwrap();
                let e = commands.spawn(&mut res).id();
                commands.apply(&mut res);
                res.insert(commands);
                e
            };
            if let Some(reg) = res.get_mut::<ComponentRegistry>() {
                reg.insert_default_reflected(&TypeId::of::<Transform>(), entity);
            }
            let empty = res
                .get_mut::<ome_ecs::archetype_registry::ArchetypeRegistry>()
                .unwrap()
                .get_or_create(Default::default());
            let archetypes = res
                .get_mut::<ome_ecs::archetype_registry::ArchetypeRegistry>()
                .unwrap();
            archetypes.register_entity(entity, empty);
            let next = archetypes.archetype_after_add::<Transform>(empty);
            archetypes.register_entity(entity, next);

            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let mut editor = ecs();
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(&socket));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");
        let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
        state.mirror.apply(&snapshot, &mut editor);
        let source = state.mirror.local_of(snapshot[0].id).expect("mirrored");

        // Give the source a distinctive value so the copy can be told apart
        // from an empty entity that merely exists.
        if let Some(reg) = editor.get_mut::<ComponentRegistry>() {
            let _ = reg.reflect_set_field(
                &TypeId::of::<Transform>(),
                source,
                "position",
                ReflectValue::Vec3(glam::Vec3::new(3.0, 4.0, 5.0)),
            );
        }
        editor.insert(state);

        let before = RemoteClient::new(&socket).list_entities().unwrap().len();
        assert!(dispatch(&mut editor, &EditorAction::Duplicate(source)));

        let entities = RemoteClient::new(&socket).list_entities().unwrap();
        assert_eq!(
            entities.len(),
            before + 1,
            "no copy was created on the project"
        );
        let copy = entities
            .iter()
            .find(|e| e.id != snapshot[0].id)
            .expect("copy not found");
        let position = copy
            .components
            .iter()
            .find(|c| c.type_name.ends_with("Transform"))
            .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
            .map(|(_, v)| v.clone());
        assert_eq!(
            position,
            Some(ReflectValue::Vec3(glam::Vec3::new(3.0, 4.0, 5.0))),
            "the copy did not carry the source's field values"
        );

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }
}
