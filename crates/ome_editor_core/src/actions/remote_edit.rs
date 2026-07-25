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
    let Some(edit) = classify(action) else {
        return false;
    };

    // Lifted out of Resources so the send can borrow the session and the
    // rest of the world at the same time, and record play state after.
    let Some(mut state) = resources.remove::<RemoteState>() else {
        return true;
    };
    let playing = matches!(edit, Edit::SetPlaying(playing) if playing);
    let is_play_toggle = matches!(edit, Edit::SetPlaying(_));

    if let Some(session) = state.session.as_ref() {
        let names = resources.get::<ComponentNames>();
        match send(edit, session, &state.mirror, names, resources) {
            Ok(()) if is_play_toggle => state.playing = playing,
            Ok(()) => {}
            Err(e) => tracing::warn!("remote edit dropped: {e}"),
        }
    }

    resources.insert(state);
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
    Spawn {
        name: Option<String>,
    },
    /// Every field of a `Transform`, from a gizmo drag.
    TransformEdit {
        entity: ome_ecs::entity::Entity,
        transform: ome_ecs::transform::Transform,
    },
    /// Write the project's world to a scene file, or replace it from one.
    SaveScene,
    LoadScene,
    /// Start or stop the project's gameplay systems in place.
    SetPlaying(bool),
}

/// Reduces an action to an [`Edit`], or `None` if remote mode does not
/// own it.
fn classify(action: &EditorAction) -> Option<Edit<'_>> {
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
        EditorAction::Spawn { name, .. } => Some(Edit::Spawn { name: name.clone() }),
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
        // Play runs the project's systems in the project we are already
        // driving, instead of launching a second copy of it.
        EditorAction::Play => Some(Edit::SetPlaying(true)),
        EditorAction::Stop => Some(Edit::SetPlaying(false)),
        // Not something remote mode owns (project mgmt, settings, …).
        _ => None,
    }
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
            .set_field(remote(entity)?, &name(component)?, field, value.clone())
            .map_err(map_err),
        Edit::AddComponent { entity, component } => client
            .add_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::RemoveComponent { entity, component } => client
            .remove_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::Despawn(entity) => client.despawn(remote(entity)?).map_err(map_err),
        Edit::Spawn { name } => client.spawn(name.as_deref()).map(|_| ()).map_err(map_err),
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
    }
}

#[cfg(test)]
mod tests {
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
        let server = (0..8)
            .find_map(|i| RemoteServer::start(17760 + i).ok())
            .expect("bind");
        let port = server.port();
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
        state.session = Some(RemoteSession::attach(port));
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
        let client = RemoteClient::new(port);
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
        let server = (0..8)
            .find_map(|i| RemoteServer::start(17780 + i).ok())
            .expect("bind");
        let port = server.port();
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
        state.session = Some(RemoteSession::attach(port));
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

        let server = (0..8)
            .find_map(|i| RemoteServer::start(17820 + i).ok())
            .expect("bind");
        let port = server.port();
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
        state.session = Some(RemoteSession::attach(port));
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
        let client = RemoteClient::new(port);
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
}
