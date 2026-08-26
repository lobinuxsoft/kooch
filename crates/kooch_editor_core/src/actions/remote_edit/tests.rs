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
        "kooch_test_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

use std::any::TypeId;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use kooch_remote::RemoteClient;
use kooch_remote::handlers::handle;
use kooch_remote::protocol::{Method, Request};
use kooch_remote::server::RemoteServer;

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
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
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
        kooch_core::run_state::Playing::is_playing(&res)
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
        .expect("kooch_editor_core is not two levels below the engine root")
        .to_path_buf()
}

/// An editor world that can resolve mesh assets out of `assets/`.
fn editor_with_assets() -> Resources {
    use kooch_core::asset_database::AssetDatabase;
    use kooch_core::asset_loader::AssetServer;
    use kooch_ecs::mesh_renderer::MeshRenderer;

    let mut r = ecs();
    {
        let reg = r.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<MeshRenderer>();
    }
    let mut server = AssetServer::new().with_asset_root(engine_root().join("assets"));
    server.register_loader::<kooch_render::meshlet::MeshletMesh, _>(
        kooch_render::meshlet::MeshletMeshLoader,
    );
    r.insert(server);
    r.insert(AssetDatabase::new());
    r.insert(kooch_core::assets::Assets::<
        kooch_render::meshlet::MeshletMesh,
    >::new());
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
    use kooch_ecs::mesh_renderer::MeshRenderer;

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
    use kooch_ecs::directional_light::DirectionalLight;

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
        into: crate::actions::SpawnTarget::Active,
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
    use kooch_ecs::hierarchy::Parent;

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();
    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);

    let main_loop = std::thread::spawn(move || {
        let mut res = ecs();
        {
            let reg = res.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Parent>();
            reg.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
        }
        // Two root entities for the editor to relate.
        for name in ["Parent", "Child"] {
            handle(
                &Request {
                    id: 1,
                    method: Method::Spawn {
                        name: Some(name.to_owned()),
                        scene: None,
                        parent: None,
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
        reg.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
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
            .get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
            .unwrap()
            .get_or_create(Default::default());
        let archetypes = res
            .get_mut::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
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

/// The bug #811 was filed for: with a project open, Ctrl+Z was
/// discarded on the first line of `dispatch` and the field kept the
/// value it had just been given.
#[test]
fn an_undone_field_goes_back() {
    let transform_ty = std::any::type_name::<Transform>();
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
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
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
    let local = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    let comp = editor
        .get_mut::<ComponentNames>()
        .unwrap()
        .intern(transform_ty);
    editor.insert(state);

    let moved = glam::Vec3::new(9.0, 9.0, 9.0);
    assert!(dispatch(
        &mut editor,
        &EditorAction::SetField {
            entity: local,
            component: comp,
            field: "position".into(),
            value: ReflectValue::Vec3(moved),
        }
    ));
    assert_eq!(position(&socket), Some(ReflectValue::Vec3(moved)));

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(glam::Vec3::ZERO)),
        "the undo did not reach the project",
    );

    // And back again — redo is the same machinery run the other way.
    assert!(dispatch(
        &mut editor,
        &EditorAction::Redo(crate::history::Document::World)
    ));
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(moved)),
        "the redo did not reach the project",
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Undoing a despawn is a creation: the entity comes back with its
/// components and values, under a new id the project hands out.
#[test]
fn an_undone_despawn_rebuilds_it() {
    let transform_ty = std::any::type_name::<Transform>();
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
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
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
        handle(
            &Request {
                id: 2,
                method: Method::SetField {
                    entity: hero,
                    component: transform_ty.into(),
                    field: "position".into(),
                    value: ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
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
    let local = state.mirror.local_of(snapshot[0].id).expect("mirrored");
    editor.insert(state);

    assert!(dispatch(&mut editor, &EditorAction::Despawn(local)));
    assert!(
        RemoteClient::new(&socket)
            .list_entities()
            .unwrap()
            .is_empty(),
        "the despawn did not reach the project",
    );

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    let entities = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(entities.len(), 1, "the entity did not come back");
    assert_eq!(
        position(&socket),
        Some(ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0))),
        "it came back without its values",
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// The first entity's `Transform.position`, as the project holds it.
fn position(socket: &str) -> Option<ReflectValue> {
    RemoteClient::new(socket)
        .list_entities()
        .unwrap()
        .first()?
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Transform"))
        .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
        .map(|(_, v)| v.clone())
}

/// Ctrl+V builds the clipboard on the project's side, and Ctrl+Z takes
/// back exactly what it built — not the entity it was copied from.
#[test]
fn a_paste_is_built_and_undone() {
    let transform_ty = std::any::type_name::<Transform>();
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
                    scene: None,
                    parent: None,
                },
            },
            &mut res,
        )
        .payload
        {
            kooch_remote::protocol::ResponsePayload::Result(
                kooch_remote::protocol::ResponseData::Spawned { entity },
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
    editor.insert(state);

    // What Ctrl+C leaves behind, without going through the panel that
    // fills it.
    let mut clipboard = crate::clipboard::EntityClipboard::default();
    clipboard.set(vec![crate::actions::entity_state::capture(&editor, source)]);
    editor.insert(clipboard);

    assert!(dispatch(&mut editor, &EditorAction::PasteEntities));
    let entities = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(entities.len(), 2, "the paste did not reach the project");
    assert!(
        entities.iter().any(
            |e| e.components.iter().any(|c| c.type_name.ends_with("Name")
                && c.fields
                    .iter()
                    .any(|(_, v)| *v == ReflectValue::String("Hero Copy".to_owned())))
        ),
        "the pasted entity was not named after its source",
    );

    assert!(dispatch(
        &mut editor,
        &EditorAction::Undo(crate::history::Document::World)
    ));
    let after = RemoteClient::new(&socket).list_entities().unwrap();
    assert_eq!(after.len(), 1, "undoing the paste took the wrong entity");
    assert_eq!(after[0].id, snapshot[0].id, "it took the original");

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// Saving one scene is the project's business, and carries which scene.
///
/// 🔴 The scene id has to survive `classify`. Dropped, the save would
/// fall through to the active scene — writing the file the user did not
/// right-click, which is a mistake nothing shows until the next load.
#[test]
fn saving_one_scene_names_it() {
    let editor = ecs();
    let id = kooch_core::Guid::new_v4();

    let named = |action| match super::classify(&action, &editor) {
        Some(super::Edit::SaveOneScene { scene, as_new }) => (scene, as_new),
        _ => panic!("a per-scene save did not classify as one"),
    };

    assert_eq!(
        named(EditorAction::SaveOpenScene(id)),
        (id, false),
        "Save lost the scene, or asked for a path instead of using the file",
    );
    assert_eq!(
        named(EditorAction::SaveOpenSceneAs(id)),
        (id, true),
        "Save As lost the scene, or wrote over the existing file without asking",
    );
}
